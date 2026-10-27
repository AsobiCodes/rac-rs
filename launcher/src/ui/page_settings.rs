//! The Settings tab: the options file of the game as plain rows (graphics, the enhanced picture, gameplay, language) and the Advanced box.
//! Every click is saved at once (atomic write of player\pc_options.txt); the game reads it at its next start.
use super::rows::{self, Group, Kind, Row};
use super::state::{App, View};
use super::theme;
use crate::gating;
use crate::pcopts::{CAMSPEED_MAX, CAMSPEED_MIN};
use crate::platform;
use egui::{RichText, ScrollArea, Ui, Vec2};

impl App {
    /// The options file the page edits: the selected profile's (the game's own file when the install has no profiles yet).
    fn options_path(&self) -> std::path::PathBuf {
        if self.profile_id.is_empty() {
            self.rac1().options_path(&self.root)
        } else {
            self.profiles.options_file(&self.profile_id)
        }
    }

    pub fn save_options(&mut self) {
        let p = self.options_path();
        if let Err(e) = self.options.store(&p) {
            self.toast(format!("Your settings could not be saved: {e}"), true);
        }
    }

    pub fn show_settings(&mut self, ui: &mut Ui) {
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.label(theme::bold("Settings", 22.0));
            ui.label(theme::muted("The same options as the game's Pause menu (Options). Your choice is saved at once in the profile and used the next time the game starts."));
            ui.add_space(4.0);
            if !self.profile_id.is_empty() {
                self.profile_header(ui);
            }
            ui.add_space(6.0);
            self.settings_presets(ui);
            self.settings_group(ui, "Graphics", Group::Graphics, None);
            let enhanced = rows::lighting_enhanced(&self.options);
            let note = if enhanced { None } else { Some("These effects are part of the Enhanced picture. Set Lighting to Enhanced above to use them.") };
            self.settings_group(ui, "Enhanced picture", Group::Enhanced, note);
            self.settings_group(ui, "Gameplay", Group::Gameplay, None);
            self.settings_group(ui, "Language", Group::Language, None);
            self.settings_advanced(ui);
            ui.add_space(12.0);
        });
    }

    fn settings_group(&mut self, ui: &mut Ui, title: &str, group: Group, note: Option<&str>) {
        ui.add_space(8.0);
        ui.label(theme::bold(title, 17.0).color(theme::ORANGE));
        if let Some(n) = note {
            ui.label(RichText::new(n).color(theme::MUTED).size(13.0));
        }
        let dim = group == Group::Enhanced && note.is_some();
        let ro = self.read_only();
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            if dim {
                ui.set_opacity(0.55);
            }
            let list: Vec<&'static Row> = rows::rows_of(group).collect();
            let n = list.len();
            for (i, row) in list.into_iter().enumerate() {
                ui.add_enabled_ui(!ro, |ui| self.settings_row(ui, row));
                if i + 1 < n {
                    ui.add_space(2.0);
                    ui.separator();
                    ui.add_space(2.0);
                }
            }
        });
    }

    /// (file value, label) of every choice of a row, in display order (rows without a fixed list give none).
    fn row_values(row: &Row) -> Vec<(&'static str, &'static str)> {
        match row.kind {
            Kind::Choice(list) => list.to_vec(),
            Kind::Camera | Kind::Percent => Vec::new(),
        }
    }

    fn row_labels(row: &Row) -> Vec<&'static str> {
        match row.kind {
            Kind::Choice(list) => list.iter().map(|(_, l)| *l).collect(),
            Kind::Camera => rows::CAMERA_LABELS.to_vec(),
            Kind::Percent => Vec::new(),
        }
    }

    fn settings_row(&mut self, ui: &mut Ui, row: &'static Row) {
        let total = ui.available_width();
        let labels = Self::row_labels(row);
        let need = match row.kind {
            Kind::Percent => f32::MAX,
            _ => theme::segmented_width(ui, &labels),
        };
        let stack = need > total * 0.5;
        let label_w = if stack { total } else { total - need - 20.0 };
        let mut picked: Option<usize> = None;
        let mut speed: Option<Option<u32>> = None;
        // values of this row that the game version ignores (per-value rows of the gating table): greyed segments, with the reason
        let values = Self::row_values(row);
        let locked: Vec<bool> = values.iter().map(|(v, _)| !gating::value_available(&self.info.local_version, row.key, v)).collect();
        let value_note = gating::value_needs_text(&self.info.local_version, row.key, &values);
        let mut control = |ui: &mut Ui, this: &App| match row.kind {
            Kind::Percent => speed = Self::percent_control(ui, this, row),
            _ => picked = theme::segmented_gated(ui, row.key, row.label, &labels, rows::current_index(&this.options, row), &locked),
        };
        // an option this game version does not know yet: greyed, with the reason
        let needs = gating::needs_text(&self.info.local_version, row.key);
        let known = needs.is_none();
        let label = |ui: &mut Ui| {
            ui.label(theme::bold(row.label, 15.5));
            ui.label(theme::muted(row.help));
            if let Some(n) = needs.as_ref().or(value_note.as_ref()) {
                ui.label(RichText::new(n.clone()).color(theme::ORANGE).size(13.0));
            }
        };
        if !known {
            ui.set_opacity(0.55);
        }
        if stack {
            ui.vertical(|ui| {
                label(ui);
                ui.add_enabled_ui(known, |ui| control(ui, self));
            });
        } else {
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(Vec2::new(label_w, 10.0), egui::Layout::top_down(egui::Align::Min), label);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.add_enabled_ui(known, |ui| control(ui, self)));
            });
        }
        if let Some(i) = picked {
            rows::choose(&mut self.options, row, i);
            self.save_options();
        }
        if let Some(s) = speed {
            rows::set_percent(&mut self.options, row.key, s);
            self.save_options();
        }
    }

    /// A percent row's slider (Camera speed, Mouse speed) and its "Use default" button. Returns the new value when the player changed it
    /// (Some(None) = back to default). Camera speed's default is the game's own Slow / Medium / Fast row, Mouse speed's the row's 50.
    fn percent_control(ui: &mut Ui, this: &App, row: &Row) -> Option<Option<u32>> {
        let cur = rows::percent(&this.options, row.key);
        let mut out = None;
        ui.horizontal(|ui| {
            let mut pct = cur.unwrap_or(row.default.parse().unwrap_or(50)) as f32;
            ui.style_mut().visuals.selection.bg_fill = theme::ORANGE;
            ui.style_mut().spacing.slider_width = 220.0;
            let r = ui.add(egui::Slider::new(&mut pct, CAMSPEED_MIN as f32..=CAMSPEED_MAX as f32).step_by(10.0).show_value(false));
            r.widget_info(|| egui::WidgetInfo::slider(true, pct as f64, row.label));
            if r.changed() {
                out = Some(Some(pct.round() as u32));
            }
            let (txt, col) = match cur {
                Some(p) => (format!("{p}%"), theme::TEXT),
                None if row.default.is_empty() => ("Game's own setting".to_string(), theme::MUTED),
                None => (format!("Default ({}%)", row.default), theme::MUTED),
            };
            ui.label(RichText::new(txt).color(col).size(14.0));
            if cur.is_some() && theme::button(ui, "Use default") {
                out = Some(None);
            }
        });
        out
    }

    fn settings_advanced(&mut self, ui: &mut Ui) {
        ui.add_space(8.0);
        ui.label(theme::bold("Advanced", 17.0).color(theme::ORANGE));
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                if theme::button(ui, "Open game folder") {
                    platform::open_path(&self.root);
                }
                if theme::button(ui, "Open game log") {
                    let log = crate::play::log_file(&self.root);
                    if log.is_file() {
                        platform::open_path(&log);
                    } else {
                        self.toast("There is no log yet. It is made the first time you play.", false);
                    }
                }
                if theme::button(ui, "Open log folder") {
                    if let Err(e) = crate::play::open_logs(&self.root) {
                        self.toast(format!("The log folder could not be opened: {e}"), true);
                    }
                }
                if theme::button(ui, "Create desktop shortcut") {
                    match self.create_shortcut() {
                        Ok(p) => self.toast(format!("Shortcut created: {}", p.display()), false),
                        Err(e) => self.toast(format!("The shortcut could not be made: {e}"), true),
                    }
                }
            });
            if !self.is_backup() {
                ui.separator();
                ui.label(theme::bold("Repair / rebuild", 15.5));
                ui.label(theme::muted("Makes the game code again from your disc and builds the game again (about 15 minutes). Use it if the game does not start after something went wrong. Your saves and settings are kept."));
                if theme::button_enabled(ui, "Repair / rebuild...", !self.game_running()) {
                    self.tab = super::state::Tab::Play;
                    self.goto_repair();
                }
            }
            ui.separator();
            ui.label(theme::bold("Reset settings", 15.5));
            ui.label(theme::muted("Puts every option above back to the game's own default. Your saves are not touched."));
            if self.confirm_reset {
                ui.horizontal(|ui| {
                    if theme::button(ui, "Yes, reset everything") {
                        self.options.reset_all();
                        self.save_options();
                        self.confirm_reset = false;
                        self.toast("Settings are back to the defaults.", false);
                    }
                    if theme::button(ui, "Cancel") {
                        self.confirm_reset = false;
                    }
                });
            } else if theme::button_enabled(ui, "Reset to defaults...", !self.read_only()) {
                self.confirm_reset = true;
            }
        });
        let _ = View::Home;
    }
}
