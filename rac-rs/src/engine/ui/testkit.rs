//! A small fake menu for the UI tests (no game): four tabs, every row kind, a dependency rule, a notice and a long tab for scrolling.
use super::kit::FixedAdvance;
use super::rows::*;
use super::Ui;

pub const fn t(s: &'static str) -> T5 {
	[s; LANGS]
}

const fn row(id: &'static str, kind: RowKind, def: i32, values: &'static [T5], cost: Cost, needs: Option<Needs>) -> UiRow {
	UiRow { id, kind, label: t(id), desc: t("What this row does."), values, def, cost, needs, note: None }
}

const NEED_LIGHT: Option<Needs> = Some(Needs { row: "lighting", min: 1, max: i32::MAX });
const LIGHT: &[T5] = &[t("Classic"), t("Enhanced")];
const SHADOWS: &[T5] = &[t("Off"), t("Low"), t("Medium"), t("High")];
const STYLE: &[T5] = &[t("Modern"), t("Classic")];
const NO_VALUES: &[T5] = &[];
const TOG: RowKind = RowKind::Toggle;
const PCT: RowKind = RowKind::Slider { min: 0, max: 100, step: 10, unit: "%" };
const CH: RowKind = RowKind::Choice;

const GFX: [UiRow; 4] = [
	row("preset", RowKind::Preset, 0, NO_VALUES, Cost::None, None),
	UiRow {
		label: ["Light", "Lumiere", "Licht", "Luz", "Luce"],
		..row("lighting", CH, 0, LIGHT, Cost::High, None)
	},
	row("shadows", CH, 3, SHADOWS, Cost::Medium, NEED_LIGHT),
	row("bloom", TOG, 1, NO_VALUES, Cost::Low, NEED_LIGHT),
];
const VIEW: [UiRow; 1] = [row("fov", RowKind::Slider { min: 60, max: 110, step: 5, unit: " deg" }, 90, NO_VALUES, Cost::None, None)];
const CAM: [UiRow; 2] = [row("sens", PCT, 50, NO_VALUES, Cost::None, None), row("invert", TOG, 0, NO_VALUES, Cost::None, None)];
const AUDIO: [UiRow; 2] = [row("music", PCT, 80, NO_VALUES, Cost::None, None), row("sfx", PCT, 100, NO_VALUES, Cost::None, None)];
const SYS_A: [UiRow; 1] = [row("menustyle", CH, 0, STYLE, Cost::None, None)];
const SYS_B: [UiRow; 11] = [
	row("o1", TOG, 0, NO_VALUES, Cost::None, None),
	row("o2", TOG, 0, NO_VALUES, Cost::None, None),
	row("o3", TOG, 0, NO_VALUES, Cost::None, None),
	row("o4", TOG, 0, NO_VALUES, Cost::None, None),
	row("o5", TOG, 0, NO_VALUES, Cost::None, None),
	row("o6", TOG, 0, NO_VALUES, Cost::None, None),
	row("o7", TOG, 0, NO_VALUES, Cost::None, None),
	row("o8", TOG, 0, NO_VALUES, Cost::None, None),
	row("o9", TOG, 0, NO_VALUES, Cost::None, None),
	row("o10", TOG, 0, NO_VALUES, Cost::None, None),
	row("o11", TOG, 0, NO_VALUES, Cost::None, None),
];

const GFX_SECS: [UiSection; 2] = [UiSection { title: t("Look"), rows: &GFX }, UiSection { title: t("View"), rows: &VIEW }];
const CAM_SECS: [UiSection; 1] = [UiSection { title: t("Camera"), rows: &CAM }];
const AUDIO_SECS: [UiSection; 1] = [UiSection { title: t("Volume"), rows: &AUDIO }];
const SYS_SECS: [UiSection; 2] = [UiSection { title: t("Menus"), rows: &SYS_A }, UiSection { title: t("Misc"), rows: &SYS_B }];
const TABS: [UiTab; 4] = [
	UiTab { id: "gfx", name: t("Graphics"), sections: &GFX_SECS },
	UiTab { id: "cam", name: t("Camera"), sections: &CAM_SECS },
	UiTab { id: "audio", name: t("Audio"), sections: &AUDIO_SECS },
	UiTab { id: "sys", name: t("System"), sections: &SYS_SECS },
];
const PRESETS: [UiPresetDef; 3] = [
	UiPresetDef { name: t("Low"), sets: &[("lighting", 0), ("shadows", 0), ("bloom", 0)] },
	UiPresetDef { name: t("Medium"), sets: &[("lighting", 1), ("shadows", 2), ("bloom", 1)] },
	UiPresetDef { name: t("High"), sets: &[("lighting", 1), ("shadows", 3), ("bloom", 1)] },
];
const NEEDS: [NeedsRule; 1] = [NeedsRule {
	row: "lighting",
	set_to: 1,
	banner: t("Most effects need Enhanced lighting"),
	detail: t("This only shows with Lighting set to Enhanced. Lighting is {} now."),
	button: t("Switch Lighting to Enhanced"),
}];
const NOTICES: [Notice; 1] = [Notice { row: "menustyle", value: 1, text: t("Classic menus from the next time") }];

pub static MENU: UiMenu = UiMenu {
	tabs: &TABS,
	presets: &PRESETS,
	preset_reset: 2,
	preset_also: &[("filter", 1)],
	needs: &NEEDS,
	notices: &NOTICES,
	chrome: UiChrome {
		title: t("Paused - Options"),
		title_front: t("Options"),
		autosave: t("Saved automatically"),
		now: t("Now"),
		default: t("Default"),
		speed_impact: t("Speed impact"),
		cost_names: [t("Low"), t("Medium"), t("High")],
		peek_note: t("Look at the game behind"),
		view_game: t("View game"),
		back_to_menu: t("Back to the menu"),
		change: t("Change"),
		back: t("Back"),
		reset: t("Reset"),
		switch_tab: t("Switch tab"),
		off_on: [t("Off"), t("On")],
		custom: t("Custom"),
	},
};

pub fn menu() -> &'static UiMenu {
	&MENU
}

pub fn new_ui() -> Ui {
	Ui::new(menu(), 0, Box::new(FixedAdvance(0.5)))
}
