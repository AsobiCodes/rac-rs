//! The modern pause menu's Achievements page (lane nach, 2026-11-03; user: "clicking achievements in the new gui redirects to
//! achievements in the old menu and nothing shows"). While the list is open (`achievements::is_open`: the Options row, F6) over a page
//! of `pause::in_set`, `pause::build` makes THIS page instead of stepping aside for the HUD-pass list: the pause shell with inert tabs,
//! one card per achievement (grade strip, name over description, grade over status: the date, n / max or Locked), a window of cards
//! that follows the focus with a scroll bar, and in a cheated session (Debug mode) a banner card first over the full list. The engine
//! hands the rows (`achievements::list_view`), the words are this pack's (5 languages). Pure: facts in, `Built` out; the input and the
//! card texts' prims are `pause_host/ach.rs`, the HUD list stays the Classic style's (and the fallback when the UI pipes are refused).
use super::pause::{branch, header, tabs, Built, Facts};
use super::title::{word, BACK};
use crate::achievements::{ListView, RowView};
use crate::achievements::Grade;
use rac_engine::ui::kit::data::{Hint, HintKind};
use rac_engine::ui::kit::page::{MenuButton, PageRow, PageScreen, Panel, Stat};
use rac_engine::ui::T5;

// Not retail text (lane nach): the page's own words. Native-speaker review wanted.
pub const BANNER: T5 = [
	"Achievements are off for this session: Debug mode was on",
	"Succ\u{e8}s d\u{e9}sactiv\u{e9}s pour cette session : le mode Debug \u{e9}tait activ\u{e9}",
	"Erfolge sind in dieser Sitzung aus: Der Debug-Modus war an",
	"Logros desactivados en esta sesi\u{f3}n: el modo Debug estaba activado",
	"Obiettivi disattivati in questa sessione: la modalit\u{e0} Debug era attiva",
];
pub const LOCKED: T5 = ["Locked", "Verrouill\u{e9}", "Gesperrt", "Bloqueado", "Bloccato"];
pub const UNLOCKED: T5 = ["Unlocked", "D\u{e9}bloqu\u{e9}", "Freigeschaltet", "Desbloqueado", "Sbloccato"];
pub const HIDDEN_NAME: T5 = ["Hidden achievement", "Succ\u{e8}s cach\u{e9}", "Verborgener Erfolg", "Logro oculto", "Obiettivo nascosto"];
pub const HIDDEN_DESC: T5 = ["Keep playing to find out", "Continuez \u{e0} jouer pour le d\u{e9}couvrir", "Spiel weiter, um es herauszufinden", "Sigue jugando para descubrirlo", "Continua a giocare per scoprirlo"];
pub const GRADES: [T5; 4] = [
	["Bronze", "Bronze", "Bronze", "Bronce", "Bronzo"],
	["Silver", "Argent", "Silber", "Plata", "Argento"],
	["Gold", "Or", "Gold", "Oro", "Oro"],
	["Platinum", "Platine", "Platin", "Platino", "Platino"],
];

/// Cards the panel holds (the kit's 80 px cards from y 166 to the hint bar); the banner takes one of them.
pub const CARDS: usize = 5;

/// What `build` reads besides the pause facts: the list and the page's focus and first visible row (`achievements::cursor`).
pub struct AchFacts {
	pub view: ListView,
	pub focus: usize,
	pub first: usize,
}

/// One visible card's own texts (drawn by `pause_host/ach.rs` over the kit's card).
#[derive(Clone, Debug, PartialEq)]
pub struct AchCard {
	/// Index into the whole list.
	pub index: usize,
	pub name: String,
	pub desc: String,
	pub grade: [f32; 3],
	pub unlocked: bool,
}

/// The page's state as built: what the host scrolls and what it draws over the kit's cards.
#[derive(Clone, Debug, PartialEq)]
pub struct AchBuilt {
	/// All rows of the list.
	pub n: usize,
	pub first: usize,
	pub focus: usize,
	/// Achievement cards the panel shows (CARDS less the banner).
	pub page: usize,
	/// The cheated session's banner (the panel's first card).
	pub banner: Option<String>,
	pub cards: Vec<AchCard>,
}

impl AchBuilt {
	/// The panel item (kit card index) of list row `i`, if it is visible.
	pub fn item_of(&self, i: usize) -> Option<usize> {
		let k = self.cards.iter().position(|c| c.index == i)?;
		Some(k + usize::from(self.banner.is_some()))
	}

	/// The list row of panel item `k` (None = the banner).
	pub fn row_at(&self, k: usize) -> Option<usize> {
		let k = k.checked_sub(usize::from(self.banner.is_some()))?;
		self.cards.get(k).map(|c| c.index)
	}
}

fn grade_index(g: Grade) -> usize {
	match g {
		Grade::Bronze => 0,
		Grade::Silver => 1,
		Grade::Gold => 2,
		Grade::Platinum => 3,
	}
}

/// The status column: the unlock date, the counter, or Unlocked / Locked.
pub fn status(r: &RowView, lang: usize) -> String {
	if r.unlocked {
		if r.at > 0 { crate::achievements::unix_to_iso(r.at)[..10].to_string() } else { word(&UNLOCKED, lang).to_string() }
	} else if r.max > 0 {
		format!("{} / {}", r.progress, r.max)
	} else {
		word(&LOCKED, lang).to_string()
	}
}

/// The card texts of row `i`: a hidden locked entry keeps its secret.
fn card_of(i: usize, r: &RowView, lang: usize) -> AchCard {
	let (name, desc) = if r.masked { (word(&HIDDEN_NAME, lang).to_string(), word(&HIDDEN_DESC, lang).to_string()) } else { (r.name.to_string(), r.desc.to_string()) };
	AchCard { index: i, name, desc, grade: r.grade.colour(), unlocked: r.unlocked }
}

/// The page for the open list. `f.view.page` is the pause page under it (its tab stays marked).
pub fn build(f: &Facts, a: &AchFacts) -> Built {
	let page = f.view.page;
	let lang = f.lang;
	let (tab_list, _, goodies) = tabs(f).unwrap_or_default();
	let sel_tab = branch(page);
	let menu: Vec<MenuButton> = tab_list.iter().enumerate().map(|(i, t)| MenuButton { text: t.text.clone(), selected: Some(i) == sel_tab, live: false }).collect();
	let v = &a.view;
	let banner = v.cheated.then(|| word(&BANNER, lang).to_string());
	let shown = CARDS - usize::from(banner.is_some());
	let n = v.rows.len();
	let focus = a.focus.min(n.saturating_sub(1));
	let first = crate::achievements::window(a.first, focus, shown, n);
	let mut rows: Vec<PageRow> = banner.iter().map(|_| PageRow::default()).collect();
	let mut cards = Vec::new();
	for i in first..n.min(first + shown) {
		let r = &v.rows[i];
		let stat = Stat { label: word(&GRADES[grade_index(r.grade)], lang).to_string(), value: status(r, lang) };
		rows.push(PageRow { stats: vec![stat], selected: i == focus, dim: !r.unlocked, ..Default::default() });
		cards.push(card_of(i, r, lang));
	}
	let pct = if v.all > 0 { v.got * 100 / v.all } else { 0 };
	let caption = (f.text)(crate::retail::pcext::achrow::ID_ACH);
	let panel = Panel::Cards { caption, side: format!("{} / {}  \u{b7}  {pct}%", v.got, v.all), cards: rows, extra: None };
	let hints = vec![Hint { kind: HintKind::Back, text: word(&BACK, lang).to_string() }];
	let screen = PageScreen { menu, panel, dialog: None, hints, device: f.device, glyphs: f.glyphs, shell: Some(header(f)), footer: String::new(), ..Default::default() }; // no footer: the page under the list's is not this page's (R22 m3)
	let n_items = cards.len() + usize::from(banner.is_some());
	Built {
		menu: vec![None; tab_list.len()],
		items: vec![None; n_items],
		buttons: Vec::new(),
		screen,
		root: false,
		page,
		sel: 0,
		goodies,
		grid: None,
		ach: Some(AchBuilt { n, first, focus, page: shown, banner, cards }),
		map_tab: tab_list.len().saturating_sub(1),
	}
}
