//! Shared pieces of the R&C1 rows table: value-name lists, the row constructor and the dependency every effect row carries.
//! Languages in every `T5`: En, Fr, De, Es, It (the game's language numbers 0..4).
use rac_engine::ui::{Cost, Needs, RowKind, UiRow, LANGS, T5};

/// The same word in all five languages (numbers, "4:3").
pub const fn t(s: &'static str) -> T5 {
	[s; LANGS]
}

pub const NO: &[T5] = &[];

/// Off (or Classic) / Low / Medium / High, the effect rows' levels.
pub const OFF: T5 = ["Off", "Non", "Aus", "No", "No"];
pub const ON: T5 = ["On", "Oui", "Ein", "S\u{ed}", "S\u{ec}"];
pub const CLASSIC: T5 = ["Classic", "Classique", "Klassisch", "Cl\u{e1}sico", "Classico"];
pub const LOW: T5 = ["Low", "Faible", "Niedrig", "Baja", "Bassa"];
pub const MEDIUM: T5 = ["Medium", "Moyen", "Mittel", "Media", "Media"];
pub const HIGH: T5 = ["High", "\u{c9}lev\u{e9}", "Hoch", "Alta", "Alta"];
pub const Q4: &[T5] = &[OFF, LOW, MEDIUM, HIGH];
pub const QC: &[T5] = &[CLASSIC, LOW, MEDIUM, HIGH];
pub const LMH: &[T5] = &[LOW, MEDIUM, HIGH];

pub const CH: RowKind = RowKind::Choice;
pub const TOG: RowKind = RowKind::Toggle;

/// Rows that show nothing until Lighting is Enhanced (DECISIONS 1, PLAN section 8).
pub const LIGHT: Option<Needs> = Some(Needs { row: "lighting", min: 1, max: i32::MAX });
/// Rows of the improved water (Waves, Underwater look): nothing at Water Classic; Water itself needs Lighting, so they dim with
/// either (a chain, `Model::blocked`).
pub const WATER: Option<Needs> = Some(Needs { row: "water", min: 1, max: i32::MAX });

#[allow(clippy::too_many_arguments)]
pub const fn row(id: &'static str, kind: RowKind, label: T5, desc: T5, values: &'static [T5], def: i32, cost: Cost, needs: Option<Needs>) -> UiRow {
	UiRow { id, kind, label, desc, values, def, cost, needs, note: None }
}

/// A slider row from `min` to `max` in `step`s with `unit` after the number.
pub const fn slider(min: i32, max: i32, step: i32, unit: &'static str) -> RowKind {
	RowKind::Slider { min, max, step, unit }
}
