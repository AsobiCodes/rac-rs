//! Sub-state the App carries besides the GPU: front end, Galactic Map, this frame's HUD values and the HUD art slots,
//! plus the overlay `sprite` quad helper.

use super::*;

/// The Galactic Map: the bundle, the explored mask, its HUD texture slot and the world box it fits.
pub(super) struct MapView {
	pub(super) bundle: map::MapBundle,
	pub(super) explored: map::Explored,
	/// The save codec's span table for this map (map bundle section 1).
	pub(super) spans: Vec<u8>,
	pub(super) slot: usize,
	pub(super) lo: [f32; 2],
	pub(super) hi: [f32; 2],
	/// The hero's map pixel and heading, for the marker.
	pub(super) hero: [f32; 2],
	pub(super) yaw: f32,
	pub(super) affine: Option<[f32; 4]>,
	/// Select-map zoom (initial 0.65, clamped 0.65..4.0) and the map pixel at the panel centre
	/// (decomp/notes/ingame_menu_leftovers.md P1-c).
	pub(super) zoom: f32,
	pub(super) pan: [f32; 2],
}

/// The front end: menu state over the looping title scene (frontend.rs, notes/title_implementation_prompt.md).
pub(super) struct FrontEnd {
	pub(super) menu: frontend::Menu,
	pub(super) text: frontend::Text,
	pub(super) art: Option<frontend::Art>,
	/// Scene tick (50 per second), looping over the scene.
	pub(super) tick: f32,
	/// Ambience: the looping bed and the timed sounds' voices.
	pub(super) bed: Vec<u64>,
	pub(super) timed: [Option<u64>; 2],
	/// UI-11c: the title attract loop, its 50 Hz tick accumulator and the playing movie.
	pub(super) att: frontend::Attract,
	pub(super) acc: f32,
	pub(super) attract_movie: Option<fmv::player::Player>,
}

/// Title ambience in the global bank [I]: looping bed sounds, then (sound, first tick, last tick) windows.
pub(super) const TITLE_BED: [usize; 3] = [2, 3, 8];
/// Language slot of all_text per menu language (EN, FR, DE, ES, IT; slot 1 is unused).
pub(super) const TEXT_SLOT: [usize; 5] = [0, 2, 3, 4, 5];

pub(super) struct HudState {
	pub(super) health: i32,
	pub(super) bolts: u32,
	pub(super) clock: f32,
	/// Bolt counter and nanotech opacity.
	pub(super) alpha: (f32, f32),
	/// Nanotech fade bytes (f1 bar, f2 orbs).
	pub(super) nano: (u8, u8),
	/// The ammo HUD: weapon shown, ammo, cap, fades f1 (bar, icon) and f2 (text); and the lock-on
	/// target in NDC.
	pub(super) ammo: Option<(u8, u16, u16, f32, f32)>,
	pub(super) reticle: Option<[f32; 2]>,
	/// NOV-14.2 RC range static: alpha and this tick's tile offsets.
	pub(super) rc_static: Option<(u8, Vec<[u8; 2]>)>,
	/// NOV-14.2b marker prompt widget: ramps (a, b) and the frame counter.
	pub(super) marker: (u8, u8, u32),
	/// HERO-3a: the breath meter's copied slot state (BRIDGE-14); None when nothing is installed.
	pub(super) breath: Option<game::breath_hud::BreathSnapshot>,
	/// ARI-15b: Skid's counter widgets, slots 5 / 7: (display, max, f1, f2) while drawn.
	pub(super) counters: [Option<(i32, i32, u8, u8)>; 2],
	/// WPN-6.2g2 / BRIDGE-39: this frame's screen sprites (FUN_0020fb40), each with its centre in window pixels.
	pub(super) sprites: Vec<(crate::game::ScreenSprite, [f32; 2])>,
	/// ARI-42c: the Trespasser puzzle HUD strips of this tick.
	pub(super) pad: Vec<crate::game::PadPrim>,
}

pub(super) fn sprite(slot: usize, rect: [f32; 4], flip: bool, a: f32) -> Quad {
	Quad { tex: slot, rect, uv: if flip { [1.0, 0.0, 0.0, 1.0] } else { [0.0, 0.0, 1.0, 1.0] }, colour: [1.0, 1.0, 1.0, a], angle: 0.0, corners: None }
}

/// The game's HUD art as overlay slots: (slot, width, height) per frame.
#[derive(Default)]
pub(super) struct HudArt {
	pub(super) bolt: Vec<(usize, f32, f32)>,
	/// The breath meter's sheet 0x753f frames 0..2 (aquarium, low-air aquarium, bubble).
	pub(super) breath: Vec<(usize, f32, f32)>,
	/// The Morph-o-Ray meter's sheet 0x7533 frames 0..1 (tank, bubble), WPN-6.7e4 (decomp/notes/weapons_framework.md).
	pub(super) morph: Vec<(usize, f32, f32)>,
	pub(super) bar: Vec<(usize, f32, f32)>,
	pub(super) orb: Vec<(usize, f32, f32)>,
	/// The Bomb Glove's item icon (0xea60 + item 10) [I].
	pub(super) glove: Vec<(usize, f32, f32)>,
	/// Ammo HUD weapon icons: frame 3 of 0xea60+id (hud.md "Ammo HUD").
	pub(super) weapon_icons: HashMap<u8, (usize, f32, f32)>,
	/// The memory-card save icon 0x755d (decomp/notes/ingame_save_quit.md s6).
	pub(super) save: Vec<(usize, f32, f32)>,
	/// Sound page volume fill 0xe99e frames 8/9, Quick Select backdrop 0xe934 frame 0 and highlight
	/// 0xe99c frame 0 (decomp/notes/ingame_menu_leftovers.md P1-d, ingame_other_screens.md s2).
	pub(super) vol: [Option<(usize, f32, f32)>; 2],
	/// Menu panel art: underlay 0xe99e frame 7, fx 0x1a, 0x1c, 0x19 (frontend::Art::frame).
	pub(super) frame: Option<[(usize, f32, f32); 4]>,
	/// Map hero marker 0xe99a frame 5 (decomp/notes/ingame_other_screens.md s1).
	pub(super) marker: Option<(usize, f32, f32)>,
	/// ARI-15b: Skid's counter icons, HUD sheets 2000 (sharks) / 2001 (nests) frame 0.
	pub(super) counters: [Option<(usize, f32, f32)>; 2],
	pub(super) wheel_bg: Option<(usize, f32, f32)>,
	pub(super) wheel_sel: Option<(usize, f32, f32)>,
	/// HelpDesk icon: fx texture record 4 (help.rs).
	pub(super) help_icon: Option<usize>,
	/// NOV-14.2 RC static: fx texture 0x1e (slot, width, height).
	pub(super) static_tex: Option<(usize, f32, f32)>,
	/// WPN-6.2g2: the screen-sprite fx textures 0x21..=0x27 (slot, width, height).
	pub(super) screen_fx: [Option<(usize, f32, f32)>; 7],
	/// ARI-42c: the Trespasser puzzle HUD fx textures 7, 16, 104, 105 (slot, width, height).
	pub(super) pad_fx: [Option<(usize, f32, f32)>; 4],
	/// Pause-menu pictures: help_ss / options_ss, the L1/R1 arrow, the map globe (frontend::Pics).
	pub(super) pics: frontend::Pics,
	/// Item icons 0xea60 + id, for the Quick Select wheel [I] and the shop's weapon rows.
	pub(super) items: HashMap<u8, (usize, f32, f32)>,
	/// Frame 2 of 0xea60 + id: the shop's ammo-row icon (FUN_002b1540).
	pub(super) ammo_items: HashMap<u8, (usize, f32, f32)>,
	/// HUD sprite 0xe935 frame 0: the shop's LED ticker sheet (FUN_002b1168).
	pub(super) led: Option<(usize, f32, f32)>,
	pub(super) font: Option<(usize, Vec<art::Glyph>)>,
	/// Level font 3 (the marker prompt's "!", NOV-14.2b).
	pub(super) font3: Option<(usize, Vec<art::Glyph>)>,
	/// Level font 2 (the shop panels' text, table boot 0x1df370).
	pub(super) font2: Option<(usize, Vec<art::Glyph>)>,
}
