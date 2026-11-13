//! Editing the values of the options screen: stepping, setting, sliders, presets, dependencies and the texts of values.
use super::model::{Change, Model};
use super::rows::{pick, Needs, NeedsRule, RowKind, UiSound};

impl Model {
	/// Is `v` a legal value for global row `i`?
	pub fn valid(&self, i: usize, v: i32) -> bool {
		let Some(r) = self.rows.get(i) else { return false };
		match r.row.kind {
			// [PC] Only values the engine offers (ui/live.rs `offer`).
			RowKind::Choice => v >= 0 && (v as usize) < r.row.values.len() && super::live::offered(r.row.id, v),
			RowKind::Toggle => v == 0 || v == 1,
			RowKind::Slider { min, max, .. } => v >= min && v <= max,
			RowKind::Preset => false,
		}
	}

	/// Sets a value (illegal values are ignored, an unchanged value is not reported). Queues the `Change`; `sound` queues Select.
	/// [PC] Unchanged = what the row shows (`value`): a row holding a withdrawn value shows its default, and a press or reset that
	/// lands on that default writes nothing, so the game keeps the choice it cannot run now (review R28 m2: the options file keeps
	/// `upscaler=fsr1` / `dlss` on a PC without the DLLs).
	fn put(&mut self, i: usize, v: i32, sound: bool) {
		if !self.valid(i, v) || self.value(i) == v {
			return;
		}
		self.vals[i] = v;
		self.touched[i] = Some(self.now);
		self.changes.push(Change { id: self.rows[i].row.id, value: v });
		if sound {
			self.sound(UiSound::Select);
		}
	}

	pub fn set_value(&mut self, i: usize, v: i32) {
		self.put(i, v, true);
	}

	/// Steps row `i` by one in direction `dir` (+1 / -1): a Choice wraps, a Toggle flips, a Slider moves by its step and stops at the
	/// ends, the Preset row goes to the next preset (from Custom to the last one).
	pub fn step_row(&mut self, i: usize, dir: i32) {
		let Some(r) = self.rows.get(i).map(|r| r.row) else { return };
		let v = self.value(i);
		match r.kind {
			RowKind::Choice => {
				// A table without values (`UiMenu::validate` reports it) stays put instead of dividing by zero. [PC] Values the engine
				// withdrew (ui/live.rs `offer`) are skipped.
				if let Some(to) = next_offered(v, dir, r.values.len() as i32, |x| super::live::offered(r.id, x)) {
					self.set_value(i, to);
				}
			}
			RowKind::Toggle => self.set_value(i, 1 - v),
			RowKind::Slider { min, max, step, .. } => self.set_value(i, (v + dir.signum() * step.max(1)).clamp(min, max.max(min))),
			RowKind::Preset => {
				let n = self.menu.presets.len() as i32;
				if n > 0 {
					let to = match self.preset_index() {
						None => n - 1,
						Some(p) => (p as i32 + dir).rem_euclid(n),
					};
					self.apply_preset(to as usize);
				}
			}
		}
	}

	pub fn step(&mut self, dir: i32) {
		self.step_row(self.focus, dir);
	}

	/// Enter / Cross / click on the focused row: next value; a Slider does nothing.
	pub fn confirm(&mut self) {
		if !matches!(self.rows.get(self.focus).map(|r| r.row.kind), Some(RowKind::Slider { .. })) {
			self.step_row(self.focus, 1);
		}
	}

	/// The focused row back to its default (the Preset row: the reset preset).
	pub fn reset(&mut self) {
		let Some(r) = self.rows.get(self.focus).map(|r| r.row) else { return };
		if r.kind == RowKind::Preset {
			self.apply_preset(self.menu.preset_reset);
		} else {
			self.set_value(self.focus, r.def);
		}
	}

	/// Slider row `i` to the position `t` (0..1) of its track, snapped to its step (while dragging: no sound).
	pub fn slider_to(&mut self, i: usize, t: f32) {
		let Some(RowKind::Slider { min, max, step, .. }) = self.rows.get(i).map(|r| r.row.kind) else { return };
		let (step, max) = (step.max(1), max.max(min)); // a step 0 / max < min table (`validate` reports it) never divides by zero
		let n = ((max - min) as f32 * t.clamp(0.0, 1.0) / step as f32).round() as i32;
		self.put(i, (min + n * step).clamp(min, max), false);
	}

	/// Position 0..1 of a Slider's value.
	pub fn slider_pos(&self, i: usize) -> f32 {
		match self.rows.get(i).map(|r| r.row.kind) {
			// min == max (`validate` reports it) is position 0, not NaN on the GPU.
			Some(RowKind::Slider { min, max, .. }) if max > min => ((self.value(i) - min) as f32 / (max - min) as f32).clamp(0.0, 1.0),
			_ => 0.0,
		}
	}

	/// The first preset whose rows all have the preset's values, None = Custom.
	pub fn preset_index(&self) -> Option<usize> {
		self.menu.presets.iter().position(|p| p.sets.iter().all(|(id, v)| self.value_of(id) == Some(*v)))
	}

	pub fn apply_preset(&mut self, p: usize) {
		let Some(def) = self.menu.presets.get(p) else { return };
		for (id, v) in def.sets {
			if let Some(i) = self.row_index(id) {
				self.put(i, *v, false);
			}
		}
		// Settings without a row: always sent (there is no value here to compare with).
		for (id, v) in self.menu.preset_also {
			self.changes.push(Change { id: *id, value: *v });
		}
		self.sound(UiSound::Select);
	}

	/// Is row `i` inert right now (its dependency is below the needed value, or the row it depends on is inert itself: a chain such
	/// as Metal reflections -> Metal -> Lighting)? An inert row is hidden (`shown`).
	pub fn blocked(&self, i: usize) -> bool {
		self.cause(i, 0).is_some()
	}

	/// Does the screen show row `i` (user 2026-11-08: rows that do nothing right now are hidden, not dimmed)?
	pub fn shown(&self, i: usize) -> bool {
		!self.blocked(i)
	}

	/// The rule of row `i`'s dependency; while the row is blocked, the rule of the link that holds it (the root of a chain first), so
	/// the box and its button fix what really hides the row. Several rules may name one row: the one whose `set_to` meets the
	/// dependency wins (an "any upscaler" row and a "DLSS only" row under one Upscaler row), else the first for that row.
	pub fn rule(&self, i: usize) -> Option<&'static NeedsRule> {
		let id = self.rows.get(i)?.row.id;
		self.rule_for(self.cause(i, 0).or_else(|| self.rows.get(i)?.row.needs)?, id)
	}

	/// The rule of dependency `n` of row `id` (see `rule`): a rule for this very row first (`for_row`: the Frame Generation row's own
	/// fix), else among the shared ones the one whose `set_to` meets `n`, else the first shared one for that row.
	fn rule_for(&self, n: Needs, id: &str) -> Option<&'static NeedsRule> {
		let of = |r: &&NeedsRule| r.row == n.row;
		let own = || self.menu.needs.iter().filter(of).find(|r| r.for_row == id);
		let shared = || self.menu.needs.iter().filter(of).filter(|r| r.for_row.is_empty());
		own().or_else(|| shared().find(|r| (n.min..=n.max).contains(&r.set_to))).or_else(|| shared().next())
	}

	/// The unmet dependency that makes row `i` inert (None = live): the cause of the row it needs first, then its own test. `depth`
	/// stops a loop (`UiMenu::validate` refuses one).
	fn cause(&self, i: usize, depth: usize) -> Option<Needs> {
		let n = self.rows.get(i)?.row.needs?;
		if depth < 8 {
			if let Some(c) = self.row_index(n.row).and_then(|j| self.cause(j, depth + 1)) {
				return Some(c);
			}
		}
		self.value_of(n.row).is_some_and(|v| v < n.min || v > n.max).then_some(n)
	}

	/// The rule of the first blocked row of the current tab that has a banner (the banner on top of the list; a rule with an empty
	/// `banner` only hides its rows).
	pub fn tab_rule(&self) -> Option<&'static NeedsRule> {
		self.cur_rows().filter(|&i| self.blocked(i)).filter_map(|i| self.rule(i)).find(|r| !r.banner[0].is_empty())
	}

	/// The fix button of a rule (the tab's banner): sets the row it names. The focus stays where it was.
	pub fn enhance(&mut self, rule: &NeedsRule) {
		if let (Some(i), Some(v)) = (self.row_index(rule.row), self.fix_value(rule)) {
			self.set_value(i, v);
		}
	}

	/// The value the fix button of `rule` sets: `set_to`, or while the engine withdrew it (ui/live.rs `offer`: FSR on a PC without
	/// FidelityFX) the next offered value after it that is not the current one (DLSS) and that still meets the dependency the rule is
	/// for (a "DLSS only" rule on a PC without DLSS: no button); None = nothing to switch to (no button).
	pub fn fix_value(&self, rule: &NeedsRule) -> Option<i32> {
		let i = self.row_index(rule.row)?;
		let want = rule.fix.map_or(rule.set_to, |f| f());
		if self.valid(i, want) {
			return Some(want);
		}
		let r = self.rows[i].row;
		let n = if r.kind == RowKind::Choice { r.values.len() as i32 } else { 0 };
		let dep = self.rows.iter().filter_map(|x| Some((x.row.needs?, x.row.id))).map(|(d, id)| (d, self.rule_for(d, id))).find(|(_, r)| r.is_some_and(|r| std::ptr::eq(r, rule))).map(|(d, _)| d);
		let meets = |v: i32| dep.map_or(true, |d| (d.min..=d.max).contains(&v));
		(1..n).map(|k| (want + k).rem_euclid(n)).find(|&v| v != self.value(i) && self.valid(i, v) && meets(v))
	}

	/// The fix button's text of `rule` (empty = no button: nothing to switch to); a `{}` in it names the value it sets.
	pub fn fix_text(&self, rule: &NeedsRule) -> String {
		let Some(v) = self.fix_value(rule) else { return String::new() };
		let name = self.row_index(rule.row).map(|i| self.text_of(i, v)).unwrap_or_default();
		crate::localization::ui_text(pick(&rule.button, self.lang)).replace("{}", crate::localization::ui_text(&name))
	}

	/// The text of value `v` of row `i`.
	pub fn text_of(&self, i: usize, v: i32) -> String {
		let Some(r) = self.rows.get(i).map(|r| r.row) else { return String::new() };
		let c = &self.menu.chrome;
		match r.kind {
			// [PC] A run-time text first (ui/live.rs: the Upscaler's FidelityFX version), else the value's own name.
			RowKind::Choice => super::live::get(r.id, v).or_else(|| r.values.get(v.max(0) as usize).map(|t| pick(t, self.lang).to_string())).unwrap_or_default(),
			RowKind::Toggle => pick(&c.off_on[(v != 0) as usize], self.lang).to_string(),
			RowKind::Slider { unit, .. } => format!("{v}{unit}"),
			RowKind::Preset => self.preset_name(self.preset_index()),
		}
	}

	/// A Choice row's offered values (ui/live.rs `offer`) and the place of `v` among them (the row's dots, screen.rs). One pass, no
	/// allocation (the screen is built every menu frame, review R28 m4).
	pub fn offered_place(&self, i: usize, v: i32) -> (usize, usize) {
		let Some(r) = self.rows.get(i).map(|r| r.row) else { return (0, 0) };
		place_among(v, r.values.len() as i32, |x| super::live::offered(r.id, x))
	}

	pub fn value_text(&self, i: usize) -> String {
		self.text_of(i, self.value(i))
	}

	pub fn default_text(&self, i: usize) -> String {
		match self.rows.get(i).map(|r| r.row) {
			Some(r) if r.kind == RowKind::Preset => self.preset_name(Some(self.menu.preset_reset)),
			Some(r) => self.text_of(i, r.def),
			None => String::new(),
		}
	}

	/// Name of a preset, or "Custom" for None.
	pub fn preset_name(&self, p: Option<usize>) -> String {
		match p.and_then(|p| self.menu.presets.get(p)) {
			Some(d) => pick(&d.name, self.lang).to_string(),
			None => pick(&self.menu.chrome.custom, self.lang).to_string(),
		}
	}
}

/// The Choice value one step from `v` in direction `dir` among `n` values (wrapping), skipping the ones `offered` refuses; None
/// for an empty table. With nothing else offered: `v` itself (no change).
pub fn next_offered(v: i32, dir: i32, n: i32, offered: impl Fn(i32) -> bool) -> Option<i32> {
	let dir = if dir < 0 { -1 } else { 1 };
	(1..=n).map(|k| (v + dir * k).rem_euclid(n)).find(|&x| offered(x))
}

/// (place of `v` among the values 0..n that `offered` accepts (0 when it is not one of them), how many there are).
fn place_among(v: i32, n: i32, offered: impl Fn(i32) -> bool) -> (usize, usize) {
	let (mut at, mut count) = (0, 0);
	for x in 0..n {
		if offered(x) {
			if x == v {
				at = count;
			}
			count += 1;
		}
	}
	(at, count)
}

#[cfg(test)]
mod offer_tests {
	use super::{next_offered, place_among};
	use crate::ui::model::Model;
	use crate::ui::rows::{Cost, RowKind, UiRow};
	use crate::ui::testkit::{menu, t};

	/// An Upscaler-like row with its own id (ui/live.rs is process-wide: no other test sees its withdrawn values).
	static UPS: UiRow = UiRow { id: "put test upscaler", kind: RowKind::Choice, label: t("Upscaler"), desc: t(""), values: &[t("Off"), t("FSR"), t("DLSS")], def: 0, cost: Cost::None, needs: None, note: None };

	/// Review R28 m2: the game kept FSR (file `upscaler=fsr1`) but FSR and DLSS cannot run here: the row shows Off, and neither a
	/// step nor a reset writes anything (the file keeps the choice); once DLSS is offered a step picks it.
	#[test]
	fn a_press_on_a_withdrawn_choice_never_writes_the_shown_default() {
		let mut m = Model::new(menu(), 0);
		let i = m.row_index("shadows").unwrap();
		m.rows[i].row = &UPS;
		m.vals[i] = 1;
		crate::ui::live::offer(UPS.id, 1, false);
		crate::ui::live::offer(UPS.id, 2, false);
		assert_eq!(m.value(i), 0, "a withdrawn value reads as the default");
		m.step_row(i, 1);
		m.step_row(i, -1);
		m.focus = i;
		m.reset();
		m.set_value(i, 1);
		assert_eq!((m.take_changes(), m.vals[i]), (vec![], 1), "nothing written, the kept choice stays");
		crate::ui::live::offer(UPS.id, 2, true);
		m.step_row(i, 1);
		assert_eq!(m.take_changes().iter().map(|c| (c.id, c.value)).collect::<Vec<_>>(), vec![(UPS.id, 2)]);
	}

	#[test]
	fn the_dots_count_only_offered_values() {
		assert_eq!(place_among(2, 3, |_| true), (2, 3));
		assert_eq!(place_among(2, 3, |x| x != 1), (1, 2), "DLSS is the 2nd of Off, DLSS");
		assert_eq!(place_among(1, 3, |x| x != 1), (0, 2), "a withdrawn value: the first dot");
		assert_eq!(place_among(0, 3, |x| x == 0), (0, 1));
		assert_eq!(place_among(0, 0, |_| true), (0, 0));
	}

	/// Lane nfsr1: the Upscaler row (Off, FSR, DLSS) on a PC without FidelityFX steps Off <-> DLSS; with neither only Off.
	#[test]
	fn stepping_skips_values_the_engine_withdrew() {
		let all = |_: i32| true;
		assert_eq!(next_offered(0, 1, 3, all), Some(1));
		assert_eq!(next_offered(0, -1, 3, all), Some(2), "wraps");
		let no_fsr = |x: i32| x != 1;
		assert_eq!(next_offered(0, 1, 3, no_fsr), Some(2));
		assert_eq!(next_offered(2, 1, 3, no_fsr), Some(0));
		assert_eq!(next_offered(2, -1, 3, no_fsr), Some(0));
		let only_off = |x: i32| x == 0;
		assert_eq!(next_offered(0, 1, 3, only_off), Some(0), "nothing else: no change");
		assert_eq!(next_offered(0, 1, 0, all), None, "an empty table stays put");
	}
}
