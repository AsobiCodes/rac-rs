//! The pause tabs from a child page, a child of `pause_host.rs` (it uses the host's private state). On the root page the tab bar is the
//! list the player walks; on every other page the game offers no tab change (the Weapons page only goes Back). A keyboard and mouse
//! player expects a click on another tab to switch at once, so a click on a tab of a child page starts a small request:
//! Back to the root (one Back per page left, each queued only when the page it leaves is the settled one; the pulses are stamped
//! with their page and dropped when the page changes), then a Select of that tab's root row. The same tab on its own main page does
//! nothing; on a sub page of that tab (Sound, Help Desk, Subtitles, Save, Load, Quit) it stops at the tab's main page.
use super::{open_map, select_quiet, State};
use crate::retail::menu_model::{resolve, submit, submit_quiet, Command, Intent};
use crate::modern_menu::pause::{branch, main_page};
use crate::retail::menu_model::pages::Page;
use rac_engine::ui::kit::Scale;

/// Fields the tab switch lets page changes skip their flight (`mouse_instant::skip_page_changes`): the Back, the Select and the open.
const SKIP_FIELDS: u32 = 90;
/// A tab switch gives up after this long (a page that never settles must not keep a request alive).
const GIVE_UP_MS: u64 = 3000;
/// A Back whose page did not change in this time is sent again (its pulse was dropped by a page change we have not seen yet).
const RETRY_MS: u64 = 500;

/// A tab switch in progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Pending {
	tab: usize,
	since: u64,
	/// The page the last Back left, and when.
	backed: Option<(Page, u64)>,
	/// A click on the achievements page (`after_list`), not checked against the page under the list yet.
	from_list: bool,
}

#[cfg(test)]
impl Pending {
	pub(super) fn test(tab: usize, since: u64) -> Pending {
		Pending { tab, since, backed: None, from_list: false }
	}
}

/// The tab under window point (x, y) on a child page without a message box; None on the root page (its tabs are the list itself).
pub(super) fn tab_at(st: &State, x: f32, y: f32) -> Option<usize> {
	let (b, l) = (st.built.as_ref()?, st.layout.as_ref()?);
	if b.root || !b.buttons.is_empty() {
		return None;
	}
	let (dx, dy) = Scale::fit(st.win.0, st.win.1).to_design(x, y);
	l.menu.iter().position(|m| m.rect.contains(dx, dy))
}

/// A click chose tab `tab` on a child page: start the switch (nothing on the tab's own main page).
pub(super) fn start(st: &mut State, tab: usize, now: u64) {
	let Some(b) = st.built.as_ref().filter(|b| !b.root && b.buttons.is_empty()) else { return };
	if tab >= b.screen.menu.len() || (branch(b.page) == Some(tab) && main_page(tab) == Some(b.page)) {
		return;
	}
	if tab == b.map_tab && b.ach.is_none() {
		return open_map(st); // Select closes the pause menu from this page too: no way back to the root first
	}
	st.pending = Some(Pending { tab, since: now, backed: None, from_list: false });
	poll(st, now);
}

/// A tab click on the achievements page (`ach.rs`), which has just closed the list (review R22 m6): it waits while the list is the
/// last frame's page, then is a tab click on the page under it (`poll`): the root selects the tab, a child page goes through `start`
/// (its own main page stays, a sub page of that tab goes back to it, another tab goes back to the root and selects; a message box
/// page has no tab change).
pub(super) fn after_list(st: &mut State, tab: usize, now: u64) {
	st.pending = Some(Pending { tab, since: now, backed: None, from_list: true });
}

/// A tab chosen on the modern map page (`map_host`, `retail::map_open::take_tab`): the map closed, the pause root came up; select it
/// there (`poll`'s root step).
pub(super) fn after_map(st: &mut State, tab: usize, now: u64) {
	st.pending = Some(Pending { tab, since: now, backed: None, from_list: false });
}

/// The Back of a tab switch: the game plays its close sound for it, but the click is one switch, so only the open sound of the new
/// tab (the Select of `poll`) is heard: the Back pulse is queued quiet (`menu_model::submit_quiet`, retail's sound call is hushed).
fn back_quiet(st: &State) {
	crate::retail::mouse_instant::skip_page_changes(SKIP_FIELDS); // the tab shows within a few fields: no 12-field page flight
	match st.snap.as_ref().and_then(|m| resolve(m, Intent::Back)) {
		Some(Command::Press(bit)) => submit_quiet(bit),
		Some(c) => submit(c),
		None => {}
	}
}

/// Every display frame (and right after a click): the next step of the switch, on the page of the last frame.
pub(super) fn poll(st: &mut State, now: u64) {
	let Some(p) = st.pending else { return };
	let Some(b) = st.built.as_ref() else { return st.pending = None };
	if now.saturating_sub(p.since) > GIVE_UP_MS {
		return st.pending = None;
	}
	if p.from_list {
		if b.ach.is_some() {
			return; // the page under the list is not the last frame's page yet
		}
		if !b.root {
			st.pending = None;
			return start(st, p.tab, now);
		}
	}
	if b.root {
		st.pending = None;
		if b.target(p.tab).is_some() {
			select_quiet(st, p.tab, now);
		}
		return;
	}
	if main_page(p.tab) == Some(b.page) {
		return st.pending = None; // the tab's own main page: a sub page's tab click ends here
	}
	let page = b.page;
	let settled = st.snap.as_ref().is_some_and(|m| !m.busy());
	if settled && p.backed.is_none_or(|(was, at)| was != page || now.saturating_sub(at) > RETRY_MS) {
		back_quiet(st);
		st.pending = Some(Pending { backed: Some((page, now)), ..p });
	}
}
