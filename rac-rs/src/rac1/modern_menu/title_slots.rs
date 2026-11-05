//! The modern New Game / Load Game slot cards (title_host.rs). Retail's slot list (item 0x1d17b0 on Load Game, 0x1d4ce8 on New Game)
//! is no row list the menu snapshot can address: `mouse_menu` reads it as `Kind::Other` (no rows, so `title::flatten` gives the cards
//! no target and a `Select` request has nothing to write) [C: RAC_MENU_LOG item dump of the Load Game page 0x1f, 2026-11-02]. So on
//! these pages the cards move the game's own cursor with pad pulses, exactly as a controller does: Up / Down step it, Cross takes the
//! selected card, and the page shows retail's cursor (`front_mirror::sel_of`, the list's word +0x40).
use crate::frontend::Page;
use crate::retail::mouse_menu::{CROSS, DOWN, UP};
use rac_engine::ui::input::Action;

/// The most steps one click queues (the pulse queue holds 8; New Game has 6 rows).
const MAX_STEPS: usize = 7;

/// A page whose rows are the slot cards.
pub fn is_cards(page: Page) -> bool {
	matches!(page, Page::NewGame | Page::LoadGame)
}

/// The pad pulse for a key or pad action on a cards page; None = not a card action (Back and the rest go on as on any page).
pub fn action_bit(a: Action) -> Option<u16> {
	match a {
		Action::Up => Some(UP),
		Action::Down => Some(DOWN),
		Action::Confirm | Action::Peek => Some(CROSS),
		_ => None,
	}
}

/// A click on card `i` while retail's cursor is on `sel`: Cross on the selected card, else the steps that take the cursor there
/// (a second click then takes it: the player sees which card is selected before it loads).
pub fn click_bits(i: usize, sel: usize) -> Vec<u16> {
	match i.cmp(&sel) {
		std::cmp::Ordering::Equal => vec![CROSS],
		std::cmp::Ordering::Greater => vec![DOWN; (i - sel).min(MAX_STEPS)],
		std::cmp::Ordering::Less => vec![UP; (sel - i).min(MAX_STEPS)],
	}
}

/// How long (ms) the steps of a click count as on their way (`click`). Retail's cursor arrives within a few fields; after this the
/// cursor is taken as it is (a dropped step must not leave the cards aiming at a card forever).
pub const AIM_MS: u64 = 1000;

/// A click on card `i` while the steps of an earlier click are still taking retail's cursor (on `sel`) to card `aim` (None = no
/// steps on their way). On the aim: nothing, its steps are queued (a double click must not step again and overshoot, R4-2); on another
/// card: the steps from the aim; the aim reached or none: `click_bits`.
pub fn click(i: usize, sel: usize, aim: Option<usize>) -> Vec<u16> {
	match aim.filter(|&a| a != sel) {
		Some(a) if a == i => Vec::new(),
		Some(a) => click_bits(i, a),
		None => click_bits(i, sel),
	}
}

/// The card to draw selected while a click's steps walk retail's cursor: the clicked card at once (`aim` = card and the ms of the
/// click) until the cursor arrives (`sel` = it) or `AIM_MS` pass; else retail's. The bar never crawls card by card.
pub fn shown(aim: Option<(usize, u64)>, sel: usize, now: u64) -> usize {
	match aim {
		Some((card, at)) if now.saturating_sub(at) < AIM_MS => card,
		_ => sel,
	}
}

/// How many of a click's steps play no sound: all but the last (the game plays one move sound per step; the click is heard once).
pub fn quiet_steps(bits: &[u16]) -> usize {
	if bits.first().is_some_and(|&b| b == CROSS) { 0 } else { bits.len().saturating_sub(1) }
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn only_new_and_load_game_are_card_pages() {
		assert!(is_cards(Page::NewGame) && is_cards(Page::LoadGame));
		assert!(!is_cards(Page::Root) && !is_cards(Page::Overwrite) && !is_cards(Page::Options) && !is_cards(Page::Sound));
	}

	#[test]
	fn keys_step_the_games_cursor_and_confirm_takes_the_card() {
		assert_eq!(action_bit(Action::Up), Some(UP));
		assert_eq!(action_bit(Action::Down), Some(DOWN));
		assert_eq!(action_bit(Action::Confirm), Some(CROSS));
		assert_eq!(action_bit(Action::Peek), Some(CROSS));
		assert_eq!(action_bit(Action::Back), None, "Back goes on to the page's own back");
		assert_eq!(action_bit(Action::Left), None);
	}

	#[test]
	fn a_click_steps_to_the_card_and_a_second_click_takes_it() {
		assert_eq!(click_bits(3, 1), vec![DOWN, DOWN]);
		assert_eq!(click_bits(0, 4), vec![UP; 4]);
		assert_eq!(click_bits(2, 2), vec![CROSS]);
		assert_eq!(click_bits(30, 0).len(), MAX_STEPS, "never more than the queue holds");
	}

	/// R4-2: a second click before retail's cursor arrived counts from where the queued steps take it.
	#[test]
	fn a_click_on_card_5_from_card_1_shows_card_5_at_once_with_one_sound() {
		let bits = click(4, 0, None);
		assert_eq!(bits, vec![DOWN; 4]);
		assert_eq!(quiet_steps(&bits), 3, "three of the four steps are hushed: one move sound");
		assert_eq!(quiet_steps(&[CROSS]), 0, "the loading Cross is never hushed");
		assert_eq!(quiet_steps(&[]), 0);
		// retail's cursor still on card 1 and walking: the bar is on card 5 from the click until the cursor arrives or AIM_MS pass
		assert_eq!(shown(Some((4, 1000)), 0, 1000), 4);
		assert_eq!(shown(Some((4, 1000)), 2, 1400), 4, "not card 2 on the way");
		assert_eq!(shown(Some((4, 1000)), 4, 1500), 4);
		assert_eq!(shown(Some((4, 1000)), 1, 1000 + AIM_MS), 1, "a dropped step lets retail's cursor win again");
		assert_eq!(shown(None, 3, 5), 3);
	}

	#[test]
	fn a_click_while_the_cursor_is_on_its_way_never_overshoots() {
		assert_eq!(click(3, 1, Some(3)), Vec::<u16>::new(), "a double click on card 3 queues no second DOWN, DOWN");
		assert_eq!(click(4, 1, Some(3)), vec![DOWN], "another card: the steps from the aim");
		assert_eq!(click(0, 1, Some(3)), vec![UP; 3]);
		assert_eq!(click(3, 3, Some(3)), vec![CROSS], "the cursor arrived: the second click takes the card");
		assert_eq!(click(2, 1, None), click_bits(2, 1));
	}
}
