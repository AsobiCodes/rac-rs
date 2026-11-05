//! [PC] The Map tab of the modern pause menu and the tab bar of the modern map page (v0.5 build 12; user 2026-11-04: "also we dont
//! have a map tab"). Retail has no Map entry in its pause menu: the planet map opens with Select (or R3) in gameplay, and Select, R3 or
//! Start close the pause menu from any of its pages, and the map from either of its pages [C: decomp/notes/ingame_menu_leftovers.md
//! P1-c; RAC_MENU_LOG run on fg1/3_0044, 2026-11-04: Select on the Weapons page 0x3 at field 220 -> mode 3 -> 0 at 222; Select at 260
//! -> mode 3 at 261, page 0xb at 263; Start on the mission list 0xc at 350 -> mode 0 at 352]. So a tab is two presses the player could
//! make, made for them:
//! * Map tab (`Goal::Map`): the pause host closes the pause menu with a Select pulse (`menu_model::cmd`) and arms this file; the worker
//!   presses Select once the game takes it again (`CALM` gameplay fields after the menu is gone), which opens the map as the
//!   player's own press would.
//! * Another tab on the map page (`Goal::Pause(tab)`): the map host closes the map with Start and arms this file; the worker presses
//!   Start, the pause menu comes up, and `pause_host` takes the tab (`take_tab`) and selects it once the root has settled.
//!
//! A press the game did not answer is made again after `RETRY` fields; a goal not reached in `GIVE_UP` fields is dropped. Nothing
//! happens unless a modern page armed it.
use std::sync::Mutex;

/// The page menu ids (`menu_keys`).
const MAP: u32 = 0xb;
const ROOT: u32 = 2;
/// The pad bits pressed.
pub const SELECT: u16 = 0x100;
pub const START: u16 = 0x800;
/// Gameplay fields with no menu before the press: retail ignores Select for the first fields after a menu closed [C: fg1/3_0044
/// Classic, 2026-11-04: the pause menu closed at field 172 (mode 3 -> 0); a Select tap at 176, 177 or 178 did nothing, one at 179
/// opened the map]. Six fields, one more for margin: the press starts in the 8th gameplay field.
const CALM: u32 = 7;
/// Fields a press holds its button.
const PRESS: u32 = 2;
/// Fields after a press with no page come up before it is made again.
const RETRY: u32 = 24;
/// Fields a goal stays armed (3 s at PAL 50 Hz).
const GIVE_UP: u32 = 150;
/// Fields the modern map page covers the screen for a Map goal (from the arming): the menu closing, `CALM`, the press and its
/// answer fit; after that the page gives way to the gameplay (the goal itself stays armed to `GIVE_UP`).
const SHOW: u32 = 40;
/// Fields the pause root's tab waits for the pause host (`take_tab`) once the root came up.
const TAB_FIELDS: u32 = 150;

/// What the armed tab wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Goal {
	/// The planet map (page 0xb).
	Map,
	/// The pause menu, then this tab of it (`modern_menu::pause` tab index).
	Pause(usize),
}

/// The state, stepped once per worker field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Opener {
	goal: Option<Goal>,
	age: u32,
	calm: u32,
	/// Fields since the last press began (None = no press made, or the retry is due).
	pressed: Option<u32>,
	/// The pause root came up for a `Goal::Pause`: its tab, and the fields it still waits for the pause host.
	tab: Option<(usize, u32)>,
	/// A field with no page menu was seen since arming: a page menu other than the wanted one after that is the player pausing
	/// again, and drops the goal (it would press Select / Start when that menu closes).
	gone: bool,
}

impl Opener {
	fn arm(&mut self, g: Goal) {
		*self = Opener { goal: Some(g), ..Opener::default() };
	}

	/// One field: `menu` = the id of the page menu the worker reads (None while there is none, and while a page flies in), `gameplay`
	/// = the game runs its play mode (mode 0, not the front end). Returns the pad bits to add to the field.
	fn field(&mut self, menu: Option<u32>, gameplay: bool) -> u16 {
		if let Some((t, left)) = self.tab {
			self.tab = left.checked_sub(1).map(|l| (t, l));
		}
		let Some(goal) = self.goal else { return 0 };
		let (want, bit) = match goal {
			Goal::Map => (MAP, SELECT),
			Goal::Pause(_) => (ROOT, START),
		};
		if menu == Some(want) {
			if let Goal::Pause(t) = goal {
				self.tab = Some((t, TAB_FIELDS));
			}
			self.goal = None;
			return 0;
		}
		if menu.is_none() {
			self.gone = true;
		} else if self.gone {
			self.goal = None;
			return 0;
		}
		self.age += 1;
		if self.age > GIVE_UP {
			self.goal = None;
			return 0;
		}
		if let Some(n) = self.pressed.as_mut() {
			*n += 1;
			if *n <= PRESS {
				return bit;
			}
			if *n < RETRY {
				return 0;
			}
			self.pressed = None;
		}
		if menu.is_some() || !gameplay {
			self.calm = 0;
			return 0;
		}
		self.calm += 1;
		if self.calm <= CALM {
			return 0;
		}
		(self.pressed, self.calm) = (Some(1), 0);
		bit
	}
}

static OPENER: Mutex<Opener> = Mutex::new(Opener { goal: None, age: 0, calm: 0, pressed: None, tab: None, gone: false });

fn lock() -> std::sync::MutexGuard<'static, Opener> {
	OPENER.lock().unwrap_or_else(|e| e.into_inner())
}

/// Main thread: a tab asks for `goal` (the host has queued the press that closes the menu on screen).
pub fn arm(goal: Goal) {
	lock().arm(goal);
}

/// Main thread: the goal still on its way (the map page shows itself while the Map tab opens it, `host/handler/frame_map.rs`).
pub fn pending() -> Option<Goal> {
	lock().goal
}

/// Main thread: the modern map page covers the screen for a Map goal, only for the first `SHOW` fields (an unanswered Select does
/// not leave an empty page over live gameplay).
pub fn opening_map() -> bool {
	let o = lock();
	o.goal == Some(Goal::Map) && o.age <= SHOW
}

/// Main thread, the pause host on its settled root page: the tab a map page's tab bar asked for, once.
pub fn take_tab() -> Option<usize> {
	lock().tab.take().map(|t| t.0)
}

/// Worker, once per field after the queued pulses (`input.rs`): the presses of an armed goal.
pub fn field(menu: Option<u32>, gameplay: bool) -> u16 {
	let mut o = lock();
	if o.goal.is_none() && o.tab.is_none() {
		return 0;
	}
	o.field(menu, gameplay)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Steps `o` over `fields` with the same inputs; returns the bits of each field.
	fn run(o: &mut Opener, fields: usize, menu: Option<u32>, gameplay: bool) -> Vec<u16> {
		(0..fields).map(|_| o.field(menu, gameplay)).collect()
	}

	#[test]
	fn the_map_tab_presses_select_once_the_pause_menu_is_gone() {
		let mut o = Opener::default();
		o.arm(Goal::Map);
		assert_eq!(run(&mut o, 3, Some(ROOT), false), [0, 0, 0], "the pause menu is still up: its own Select pulse closes it");
		assert_eq!(run(&mut o, 1, None, false), [0], "closing (mode 3): no press");
		let mut want = vec![0; CALM as usize];
		want.extend([SELECT, SELECT, 0]);
		assert_eq!(run(&mut o, CALM as usize + 3, None, true), want, "CALM gameplay fields (retail ignores Select then), then Select for two fields");
		assert_eq!(run(&mut o, 2, None, false), [0, 0], "the map flies in (no menu read, mode 3): no second press");
		assert_eq!(o.field(Some(MAP), false), 0);
		assert_eq!(o.goal, None, "the map is up: done");
		assert_eq!(o.field(None, true), 0);
	}

	#[test]
	fn an_unanswered_press_is_made_again_and_the_goal_gives_up() {
		let mut o = Opener::default();
		o.arm(Goal::Map);
		let bits = run(&mut o, RETRY as usize + 2 * CALM as usize + 4, None, true);
		assert_eq!(bits.iter().filter(|&&b| b == SELECT).count(), 2 * PRESS as usize, "the press, and after RETRY fields its retry");
		let mut o = Opener::default();
		o.arm(Goal::Map);
		run(&mut o, GIVE_UP as usize + 1, None, false);
		assert_eq!(o.goal, None, "never reached gameplay: dropped");
	}

	#[test]
	fn another_tab_on_the_map_presses_start_and_hands_its_tab_to_the_root() {
		let mut o = Opener::default();
		o.arm(Goal::Pause(4));
		assert_eq!(run(&mut o, 2, Some(MAP), false), [0, 0], "the map is still up: the host's Start closes it");
		let mut want = vec![0; CALM as usize];
		want.extend([START, START]);
		assert_eq!(run(&mut o, CALM as usize + 2, None, true), want);
		assert_eq!(o.field(Some(ROOT), false), 0);
		assert_eq!((o.goal, o.tab.map(|t| t.0)), (None, Some(4)));
		run(&mut o, TAB_FIELDS as usize + 1, Some(ROOT), false);
		assert_eq!(o.tab, None, "a tab the pause host never took expires");
	}

	/// Review B LOW-3: the page is shown only for the first SHOW fields; a pause again after the menu was gone drops the goal.
	#[test]
	fn an_unanswered_map_press_stops_covering_gameplay_and_a_second_pause_drops_the_goal() {
		let mut o = Opener::default();
		o.arm(Goal::Map);
		run(&mut o, SHOW as usize + 1, None, true);
		assert!(o.goal == Some(Goal::Map) && o.age > SHOW, "still armed, but past the show window");
		let mut o = Opener::default();
		o.arm(Goal::Map);
		run(&mut o, 3, Some(ROOT), false);
		run(&mut o, 2, None, false);
		assert_eq!(o.field(Some(ROOT), false), 0);
		assert_eq!(o.goal, None, "the player paused again before the press: nothing fires when that menu closes");
		assert_eq!(run(&mut o, CALM as usize + 3, None, true), vec![0; CALM as usize + 3]);
	}

	#[test]
	fn the_statics_hand_the_tab_out_once() {
		arm(Goal::Pause(2));
		assert_eq!(pending(), Some(Goal::Pause(2)));
		assert_eq!(field(Some(ROOT), false), 0);
		assert_eq!(pending(), None);
		assert_eq!((take_tab(), take_tab()), (Some(2), None));
		assert_eq!(field(None, true), 0, "nothing armed: no press");
	}
}
