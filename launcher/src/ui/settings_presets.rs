//! The "Graphics quality" selector at the top of the Settings page: Low / Medium / High set the picture options in one click, Custom shows
//! by itself when the options match none of them. The values are in `presets::TABLE`.
use super::presets::{self, Preset};
use super::state::App;
use super::theme;
use egui::{RichText, Ui, Vec2};

const HELP: &str = "Sets the picture options below in one click. Low and Medium turn the costly ones down for slower PCs; High is the game's own default. Change any option afterwards and this shows Custom.";

impl App {
    /// What the selector shows: the preset the player clicked while the options still match it, else the preset the options are, else Custom.
    pub fn shown_preset(&self) -> Preset {
        let v = &self.info.local_version;
        match self.preset_picked {
            Some(p) if presets::matches(&self.options, v, p) => p,
            _ => presets::detect(&self.options, v),
        }
    }

    /// The player clicked a preset: its values are written into the options file at once. Custom is only a status.
    pub fn choose_preset(&mut self, p: Preset) {
        if p == Preset::Custom {
            return;
        }
        let v = self.info.local_version.clone();
        presets::apply(&mut self.options, &v, p);
        self.preset_picked = Some(p);
        self.save_options();
    }

    pub fn settings_presets(&mut self, ui: &mut Ui) {
        let shown = self.shown_preset();
        let note = presets::version_note(&self.info.local_version);
        let labels: Vec<&str> = Preset::ALL.iter().map(|p| p.label()).collect();
        // Custom is a status, not a button: greyed until the options are custom
        let locked = [false, false, false, shown != Preset::Custom];
        let selected = Preset::ALL.iter().position(|p| *p == shown);
        ui.add_space(8.0);
        ui.label(theme::bold("Graphics quality", 17.0).color(theme::ORANGE));
        let mut picked = None;
        let ro = self.read_only();
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            let total = ui.available_width();
            let need = theme::segmented_width(ui, &labels);
            let stack = need > total * 0.5;
            let label_w = if stack { total } else { total - need - 20.0 };
            let text = |ui: &mut Ui| {
                ui.label(theme::bold("Preset", 15.5));
                ui.label(theme::muted(HELP));
                if let Some(n) = &note {
                    ui.label(RichText::new(n.clone()).color(theme::ORANGE).size(13.0));
                }
            };
            ui.add_enabled_ui(!ro, |ui| {
                if stack {
                    ui.vertical(|ui| {
                        text(ui);
                        picked = theme::segmented_gated(ui, "preset", "Graphics quality", &labels, selected, &locked);
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(Vec2::new(label_w, 10.0), egui::Layout::top_down(egui::Align::Min), text);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            picked = theme::segmented_gated(ui, "preset", "Graphics quality", &labels, selected, &locked);
                        });
                    });
                }
            });
        });
        if let Some(i) = picked {
            self.choose_preset(Preset::ALL[i]);
        }
    }
}
