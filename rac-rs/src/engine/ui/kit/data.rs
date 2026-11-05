//! The plain data a screen hands the kit: strings, numbers and flags, nothing about options or about a game. A screen (the
//! options screen is the first, `ui/screen.rs`; later the pause menu, the title menu, the vendor, the ship travel menu) fills a
//! `Screen` from its own model every frame; the kit lays it out (`layout`), hit-tests it (`hit`) and draws it (`draw`).
use crate::ui::input::{Device, GlyphSet};
use crate::ui::prim::Rgba;
use crate::ui::theme;

/// What a hint-bar entry (or a button's lead) stands for; the keycap or pad glyph follows from the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintKind {
	Change,
	Back,
	Reset,
	Peek,
	/// The two tab-switch keys (Q E / L1 R1).
	Tabs,
}

impl HintKind {
	/// Reset and Peek are clickable buttons in the hint bar.
	pub fn is_button(self) -> bool {
		matches!(self, HintKind::Reset | HintKind::Peek)
	}
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hint {
	pub kind: HintKind,
	pub text: String,
}

/// One chip of a chips control (the preset row: Low Medium High Custom). A disabled chip is shown outlined and cannot be picked.
#[derive(Clone, Debug, PartialEq)]
pub struct Chip {
	pub text: String,
	pub selected: bool,
	pub disabled: bool,
}

/// The control on the right of a row.
#[derive(Clone, Debug, PartialEq)]
pub enum Control {
	None,
	/// Chevron, value with pips (`index` of `count`), chevron.
	Choice { value: String, index: usize, count: usize },
	/// Value text and a switch.
	Toggle { value: String, on: bool },
	/// Slider with the value text; `t` = 0..1 position of the thumb.
	Slider { value: String, t: f32 },
	Chips(Vec<Chip>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct RowData {
	pub label: String,
	/// The label is drawn dimmed (the row is inert right now).
	pub dim: bool,
	pub control: Control,
}

/// A box at the top of the list: a sentence and one button (the "needs Enhanced lighting" banner).
#[derive(Clone, Debug, PartialEq)]
pub struct BannerData {
	pub text: String,
	pub button: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
	Banner(BannerData),
	/// A section title (drawn upper-cased).
	Section(String),
	Row(RowData),
}

/// The meter row of the detail panel: label, `level` of `max` bars, a word.
#[derive(Clone, Debug, PartialEq)]
pub struct Meter {
	pub label: String,
	pub level: u8,
	pub max: u8,
	pub text: String,
}

/// The amber box in the detail panel: a sentence and one button.
#[derive(Clone, Debug, PartialEq)]
pub struct BoxData {
	pub text: String,
	pub button: String,
}

/// The bottom row of the detail panel: a note and a button with the lead of `lead` (the "View game" button).
#[derive(Clone, Debug, PartialEq)]
pub struct Footer {
	pub note: String,
	pub button: String,
	pub lead: HintKind,
}

/// The right panel: path, title, Now / Default, body, optional note, box, meter and footer.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Detail {
	pub path: String,
	pub title: String,
	/// (label, value) pairs of the Now / Default line.
	pub now: Option<(String, String)>,
	pub default: Option<(String, String)>,
	pub body: String,
	pub note: Option<String>,
	pub boxed: Option<BoxData>,
	pub meter: Option<Meter>,
	pub footer: Option<Footer>,
}

/// A whole screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
	/// Small caption above the tabs ("PAUSED · OPTIONS").
	pub title: String,
	/// Right of the header, below the notice.
	pub status: String,
	/// An accent-coloured pill above the status line.
	pub notice: Option<String>,
	pub tabs: Vec<String>,
	pub tab: usize,
	pub items: Vec<Item>,
	/// Index in `items` of the focused row.
	pub focus: Option<usize>,
	/// Scroll offset of the list in design units.
	pub scroll: f32,
	pub detail: Option<Detail>,
	pub hints: Vec<Hint>,
	pub device: Device,
	pub glyphs: GlyphSet,
	/// Panels hidden: only the backdrop (nearly clear) and the pill are drawn.
	pub peek: bool,
	/// Text of the pill while peeking ("Back to the menu").
	pub peek_text: String,
	pub accent: Rgba,
}

impl Default for Screen {
	fn default() -> Screen {
		Screen {
			title: String::new(),
			status: String::new(),
			notice: None,
			tabs: Vec::new(),
			tab: 0,
			items: Vec::new(),
			focus: None,
			scroll: 0.0,
			detail: None,
			hints: Vec::new(),
			device: Device::Keyboard,
			glyphs: GlyphSet::PlayStation,
			peek: false,
			peek_text: String::new(),
			accent: theme::ACCENT,
		}
	}
}
