//! Which retail page is on screen, by name: the page id (+0x3c of the page struct) of the in-level menu [C: decomp/notes/
//! ingame_pause_menu.md], the front end's pages by their struct address (New Game and Load Game share id 0x1f), and the modern
//! screen each page belongs to (`hide::Screen`).
use super::addrs::{P_LANGUAGE, P_LOAD, P_NEW, P_OPTIONS, P_OVERWRITE, P_ROOT, P_SOUND, P_VIDEO};
use crate::modern_menu::hide::Screen;
use crate::retail::mouse_menu::{Kind, MenuSnap};
use crate::retail::{disc, menu_keys, pcext, pcmods};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
	// The title front end.
	TitleRoot,
	TitleNew,
	TitleLoad,
	TitleOverwrite,
	TitleOptions,
	TitleLanguage,
	TitleVideo,
	TitleSound,
	/// [PC] the Graphics page and the Mods page (reserved RAM, the same address on both discs).
	TitleGraphics,
	TitleMods,
	// The in-level pages.
	Root,
	Weapons,
	Gadgets,
	QuickSelect,
	Items,
	Help,
	Options,
	Goodies,
	PauseMap,
	Missions,
	QuitGame,
	GalaxyMap,
	GalaxyDetails,
	HelpDesk,
	HelpLog,
	Controls,
	Moves,
	HelpWeapons,
	HelpGadgets,
	Overwrite,
	Subtitles,
	Camera,
	/// [PC] the Controls page and the Mods page of the pause Options list: copies of the Camera page (id 0x1d) at their own address (`pcpage::kind_of`).
	PcControls,
	PcMods,
	Save,
	Load,
	Sound,
	PostGame,
	SkillPoints,
	Credits,
	Cheats,
	Cinematics,
	InLevelMovies,
	Sketchbook,
	Epilogue,
	Commercials,
	/// A page id this table does not know.
	Other(u32),
}

/// The in-level page ids.
const IDS: [(u32, Page); 34] = [
	(menu_keys::ROOT, Page::Root),
	(3, Page::Weapons),
	(4, Page::Gadgets),
	(5, Page::QuickSelect),
	(6, Page::Items),
	(7, Page::Help),
	(8, Page::Options),
	(9, Page::Goodies),
	(menu_keys::MAP, Page::PauseMap),
	(menu_keys::MISSIONS, Page::Missions),
	(menu_keys::QUIT_GAME, Page::QuitGame),
	(menu_keys::GALAXY, Page::GalaxyMap),
	(menu_keys::GALAXY_CHILD, Page::GalaxyDetails),
	(0x13, Page::HelpDesk),
	(0x15, Page::HelpLog),
	(0x16, Page::Controls),
	(0x17, Page::Moves),
	(0x18, Page::HelpWeapons),
	(0x19, Page::HelpGadgets),
	(menu_keys::OVERWRITE, Page::Overwrite),
	(0x1b, Page::Subtitles),
	(0x1d, Page::Camera),
	(0x1e, Page::Save),
	(0x1f, Page::Load),
	(0x20, Page::Sound),
	(0x22, Page::PostGame),
	(0x24, Page::SkillPoints),
	(0x25, Page::Credits),
	(0x26, Page::Cheats),
	(0x27, Page::Cinematics),
	(0x28, Page::InLevelMovies),
	(0x29, Page::Sketchbook),
	(0x2a, Page::Epilogue),
	(0x2c, Page::Commercials),
];

impl Page {
	/// The in-level page of a page id.
	pub fn from_id(id: u32) -> Page {
		IDS.iter().find(|r| r.0 == id).map_or(Page::Other(id), |r| r.1)
	}

	/// The page id of an in-level page (None for the title's pages).
	pub fn id(self) -> Option<u32> {
		match self {
			Page::Other(id) => Some(id),
			Page::PcControls | Page::PcMods => Some(0x1d),
			p => IDS.iter().find(|r| r.1 == p).map(|r| r.0),
		}
	}

	pub fn is_title(self) -> bool {
		self.id().is_none()
	}

	/// The screen whose switch hides the retail picture of this page: the title's pages, the ship's maps, the planet map and its
	/// mission list, the rest of the pause menu.
	pub fn screen(self) -> Screen {
		match self {
			p if p.is_title() => Screen::Title,
			Page::GalaxyMap | Page::GalaxyDetails => Screen::Ship,
			Page::PauseMap | Page::Missions => Screen::Map,
			_ => Screen::Pause,
		}
	}
}

/// The title page whose struct lives at `page` (a RAM address) or, for an address this table does not know, by the page id.
fn front_page(m: &MenuSnap) -> Page {
	if m.page == pcext::TGPAGE {
		return Page::TitleGraphics;
	}
	if m.page == pcmods::TPAGE {
		return Page::TitleMods;
	}
	match disc::unva(m.page) {
		P_ROOT => Page::TitleRoot,
		P_NEW => Page::TitleNew,
		P_OVERWRITE => Page::TitleOverwrite,
		P_LOAD => Page::TitleLoad,
		p if P_OPTIONS.contains(&p) => Page::TitleOptions,
		P_LANGUAGE => Page::TitleLanguage,
		P_VIDEO => Page::TitleVideo,
		P_SOUND => Page::TitleSound,
		_ => match m.id {
			0x2e => Page::TitleRoot,
			0x2f => Page::TitleOptions,
			0x30 => Page::TitleLanguage,
			0x31 => Page::TitleVideo,
			0x1a => Page::TitleOverwrite,
			id => Page::Other(id),
		},
	}
}

/// The in-level page of a snapshot: its id, except that the PC Controls and Mods pages (copies of the Camera page) are told apart by address.
fn in_level_page(m: &MenuSnap) -> Page {
	match (Page::from_id(m.id), super::pcpage::kind_of(m.page)) {
		(Page::Camera, Some(k)) => k.page(),
		(p, _) => p,
	}
}

/// Where a menu snapshot stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
	pub page: Page,
	pub screen: Screen,
	/// The page has settled (not flying in or closing): a command now reaches it.
	pub ready: bool,
	/// The Quick Select page with its ring up (the ring draws instead of a list).
	pub ring: bool,
}

pub fn route(m: &MenuSnap) -> Route {
	let page = if m.front { front_page(m) } else { in_level_page(m) };
	let screen = if m.front { Screen::Title } else { page.screen() };
	let ring = page == Page::QuickSelect && m.items.iter().any(|i| matches!(i.kind, Kind::Ring { .. }));
	Route { page, screen, ready: !m.busy(), ring }
}
