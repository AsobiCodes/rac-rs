//! The breath meter (HERO-3a / BRIDGE-14, decomp/notes/hud.md "S1 breath HUD"): HUD slot 4, an aquarium that shows the
//! smoothed breath counter as 100 drifting bubbles. The slot's lifetime / slide state, the smoothed value and the
//! bubble positions persist across ticks here (`Game.breath_hud`); the viewer only draws a copied snapshot and never
//! advances it or draws from the shared RNG.


/// Slide curves (f32 bit patterns of the tables at 0x17e7a0 / 0x17e800), 24 entries each.
pub const SHOW: [u32; 24] = [
	1065353216, 1064641795, 1063822295, 1062891646, 1061848388, 1060692069, 1059422067, 1058036169, 1056094642, 1052828957, 1049315238, 1042663441,
	1029217509, 3173238608, 3183157566, 3181163359, 3155035194, 1022470115, 3148755415, 3122378410, 3134426330, 985637012, 973898330, 3106528907,
];
pub const HIDE: [u32; 24] = [
	1065353216, 1064642349, 1063825650, 1062902987, 1061878386, 1060760805, 1059564875, 1058311315, 1057026717, 1054520906, 1052024288, 1049670848,
	1046479318, 1042724443, 1038963326, 1033933920, 1028468037, 1022731034, 1015584209, 1007401222, 998379107, 987337819, 975272719, 961587880,
];

/// Counter maximum (slot max) and the bubble count.
pub const MAX: i32 = 10000;
/// Blink period and on-phase length in ticks (PAL).
const BLINK: u32 = 60;
const BLINK_ON: u32 = 37;


/// Bubble radius used by the motion and the draw (a different second boundary).
pub fn radius(i: usize) -> f32 {
	if i < 10 {
		6.0
	} else if i < 30 {
		4.5
	} else {
		3.0
	}
}

/// What the renderer needs of the slot (a copy; drawing never advances the state).
#[derive(Clone, Debug)]
pub struct BreathSnapshot {
	pub display: i32,
	pub slide: i32,
	pub lifetime: i32,
	/// The low-air blink shows frame 1 instead of frame 0.
	pub blink: bool,
	pub bubbles: Vec<[f32; 4]>,
	/// The installed callbacks are the Morph-o-Ray meter's (sheet 0x7533, WPN-6.7e4), not the breath meter's.
	pub morph: bool,
}


#[derive(Clone, Debug)]
pub struct BreathHud {
	/// The draw gate (0x17e908 or 0x15f404 set): the slot is not drawn.
	pub draw_blocked: bool,
	/// The update and draw callbacks are installed (the request was committed).
	active: bool,
	/// The installed callbacks are the Morph ones (init 0x24bd10, update 0x24bf00, draw 0x24c050).
	morph: bool,
	pub(crate) lifetime: i32,
	slide: i32,
	display: i32,
	bubbles: Vec<[f32; 4]>,
	frame: u32,
}

impl Default for BreathHud {
	fn default() -> BreathHud {
		BreathHud {
			draw_blocked: false,
			active: false,
			morph: false,
			lifetime: 0,
			slide: -6,
			display: 0,
			bubbles: Vec::new(),
			frame: 0,
		}
	}
}


impl BreathHud {


	/// Low air: the smoothed value is at most a third of the maximum.
	fn low(&self) -> bool {
		self.display * 3 < MAX + 1
	}

	fn phase(frame: u32) -> bool {
		(frame as i32) % (BLINK as i32) < BLINK_ON as i32
	}











	/// The copy the renderer draws; None while nothing is installed or the draw gate is shut.
	pub fn snapshot(&self) -> Option<BreathSnapshot> {
		(self.active && !self.draw_blocked).then(|| BreathSnapshot {
			display: self.display,
			slide: self.slide,
			lifetime: self.lifetime,
			blink: !self.morph && self.low() && Self::phase(self.frame),
			bubbles: self.bubbles.clone(),
			morph: self.morph,
		})
	}
}

