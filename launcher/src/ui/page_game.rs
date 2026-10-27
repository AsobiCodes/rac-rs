//! The game page: header with the tabs, and the Play tab (set up the game from a disc, or Play with the update banner).
use super::state::{App, DiscState, Purpose, RunKind, Tab, View};
use super::theme;
use crate::games::GameDef;
use crate::update::UpdateState;
use egui::{RichText, ScrollArea, Ui, Vec2};

impl App {
    pub fn show_game_page(&mut self, ui: &mut Ui) {
        let g = self.rac1();
        let installed = self.info.installed;
        let tabs_visible = installed && matches!(self.view, View::Home | View::Done);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(theme::bold(g.title, 26.0));
                ui.label(theme::muted(g.tagline));
            });
            if tabs_visible {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                    let cur = [Tab::Play, Tab::Settings, Tab::Mods].iter().position(|t| *t == self.tab);
                    if let Some(i) = theme::segmented(ui, "tabs", "Page", &["Play", "Settings", "Mods"], cur) {
                        let t = [Tab::Play, Tab::Settings, Tab::Mods][i];
                        if t != self.tab {
                            self.tab = t;
                            self.confirm_reset = false;
                            self.profile_ui.open = false;
                            self.reload_game_data();
                        }
                    }
                });
            }
        });
        ui.add_space(8.0);
        if tabs_visible && self.tab == Tab::Settings {
            self.show_settings(ui);
        } else if tabs_visible && self.tab == Tab::Mods {
            self.show_mods(ui);
        } else {
            match self.view {
                View::Run => self.show_run(ui),
                View::Backup => self.show_backup_progress(ui),
                View::Failed => self.show_failed(ui),
                View::Done => self.show_done(ui),
                View::Home | View::Repair => {
                    if installed && self.view == View::Home {
                        self.show_play(ui, g)
                    } else {
                        self.show_pick(ui, g)
                    }
                }
            }
        }
    }

    /// Cover on the left, the content on the right.
    fn hero(&mut self, ui: &mut Ui, g: &'static GameDef, right: impl FnOnce(&mut App, &mut Ui)) {
        let avail = ui.available_width();
        let cw = (avail * 0.32).clamp(170.0, 250.0);
        let serial = self.disc_serial().unwrap_or(g.discs[0].serial);
        let all: Vec<&str> = std::iter::once(serial).chain(g.discs.iter().map(|d| d.serial)).collect();
        ui.horizontal_top(|ui| {
            self.covers.show(ui, &all, g.title, Vec2::new(cw, cw * 1.38));
            ui.add_space(14.0);
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                right(self, ui);
            });
        });
    }

    fn show_play(&mut self, ui: &mut Ui, g: &'static GameDef) {
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            self.hero(ui, g, |this, ui| {
                this.play_status_chips(ui);
                this.update_banner(ui);
                let running = this.game_running();
                let label = if running { "Playing..." } else { "Play" };
                ui.horizontal(|ui| {
                    if !this.profile_id.is_empty() {
                        this.profile_picker(ui, true, 200.0);
                    }
                    let w = ui.available_width();
                    if theme::primary_button(ui, label, Vec2::new(w, 72.0), !running) {
                        let ctx = ui.ctx().clone();
                        this.play(&ctx);
                    }
                });
                if running {
                    ui.label(RichText::new("The game is running. Close its window to play again from here.").color(theme::GREEN).size(13.5));
                }
                if let Some(m) = this.play_error.clone() {
                    theme::card_colored(ui, theme::RED, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(m).color(theme::TEXT));
                    });
                }
                this.iso_missing_note(ui);
                this.play_footer(ui);
                this.play_summary(ui);
                this.saves_row(ui);
            });
        });
    }

    fn play_status_chips(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            theme::chip(ui, &format!("Version {}", self.info.local_version), theme::BLUE);
            if self.is_backup() {
                theme::chip(ui, "Backup", theme::ORANGE);
            }
            if let Some(d) = self.info.disc {
                theme::chip(ui, d.region, theme::MUTED);
            }
            if self.info.finish_pending() {
                theme::chip(ui, "Update unfinished", theme::ORANGE);
            }
        });
    }

    fn iso_missing_note(&mut self, ui: &mut Ui) {
        if self.info.iso.is_some() && !self.info.iso_exists {
            theme::card_colored(ui, theme::RED, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Your disc image was not found where it was during setup.").color(theme::RED));
                ui.label(theme::muted("Use Repair / rebuild (Settings, Advanced) to pick it again."));
                if !self.is_backup() && theme::button(ui, "Repair / rebuild...") {
                    self.goto_repair();
                }
            });
        }
    }

    /// A quiet card with what the next start will use (disc, picture, mods).
    fn play_summary(&mut self, ui: &mut Ui) {
        let on = self.mods_ui.store.entries.iter().filter(|e| self.mods_ui.store.is_enabled(&e.id)).count();
        let total = self.mods_ui.store.entries.len();
        let lighting = if super::rows::lighting_enhanced(&self.options) { "Enhanced" } else { "Classic (as on PlayStation 2)" };
        let disc = self.info.disc.map(|d| d.name.to_string()).unwrap_or_else(|| "unknown".to_string());
        ui.add_space(8.0);
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(theme::bold("Next start", 15.5));
            ui.add_space(2.0);
            let line = |ui: &mut Ui, k: &str, v: String| {
                ui.horizontal(|ui| {
                    ui.add_sized([110.0, 18.0], egui::Label::new(theme::muted(k)));
                    ui.label(RichText::new(v).size(14.0));
                });
            };
            if !self.profile_id.is_empty() {
                line(ui, "Profile", self.profile_name());
            }
            line(ui, "Disc", disc);
            line(ui, "Lighting", lighting.to_string());
            line(ui, "Mods", if total == 0 { "none installed".to_string() } else { format!("{on} of {total} turned on") });
        });
    }

    /// "Open saves folder" and "Open log folder" with their hint lines (the memory card file lives there; copy it to move
    /// saves between installs; the run log is what a problem report needs).
    fn saves_row(&mut self, ui: &mut Ui) {
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if theme::button(ui, "Open saves folder") {
                if let Err(e) = crate::play::open_saves(&self.root) {
                    self.toast(format!("The saves folder could not be opened: {e}"), true);
                }
            }
            if theme::button(ui, "Open log folder") {
                if let Err(e) = crate::play::open_logs(&self.root) {
                    self.toast(format!("The log folder could not be opened: {e}"), true);
                }
            }
            ui.hyperlink_to("Report a problem", crate::play::ISSUES_URL);
        });
        ui.label(theme::muted(crate::play::SAVES_HINT));
        ui.label(theme::muted(crate::play::LOG_HINT));
    }

    fn play_footer(&mut self, ui: &mut Ui) {
        if self.is_backup() {
            return;
        }
        let st = self.update_state();
        ui.horizontal(|ui| {
            match &st {
                UpdateState::Checking => ui.label(theme::muted("Checking for updates...")),
                UpdateState::UpToDate(v) => ui.label(theme::muted(format!("You have the latest version (v{v})."))),
                UpdateState::Offline => ui.label(theme::muted("Could not check for updates. Are you online?")),
                UpdateState::Available(_) => ui.label(theme::muted("")),
            };
            if !matches!(st, UpdateState::Checking) && theme::link(ui, "Check again") {
                self.check_updates();
            }
        });
    }

    fn update_banner(&mut self, ui: &mut Ui) {
        if self.is_backup() {
            self.backup_note(ui);
            return;
        }
        let UpdateState::Available(offer) = self.update_state() else { return };
        let running = self.game_running();
        if self.is_pinned() {
            // an install the user put on an older version: no banner, only a plain button
            // the sentence wraps inside the page and the button sits under it (a one-line row ran off the page in a narrow window)
            ui.label(theme::muted(format!("This install stays on version {} (you chose it). Version {} is out.", offer.from, offer.to)));
            let ready = self.backup_choice(ui, offer.finish);
            self.backup_error_note(ui);
            if theme::button_enabled(ui, &format!("Update to {}", offer.to), !running && !offer.finish && ready) {
                if let Some(id) = self.cur_id.clone() {
                    self.library.set_pinned(&id, false);
                    self.save_library();
                }
                self.begin_update(&offer.to, false);
            }
            return;
        }
        let mut go = false;
        theme::card_colored(ui, theme::ORANGE, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::chip(ui, if offer.finish { "Finish update" } else { "Update" }, theme::ORANGE);
                let title = if offer.finish { format!("Finish the update to v{}", offer.to) } else { format!("Version {} is available (you have {})", offer.to, offer.from) };
                ui.label(theme::bold(title, 16.0));
            });
            ui.label(theme::muted(if offer.finish {
                "An earlier update was not finished. It takes about 2 to 10 minutes. Your saves and settings are kept."
            } else {
                "This takes about 2 to 10 minutes. Your saves and settings are kept."
            }));
            if !offer.changes.is_empty() {
                let open = self.update_changes_open;
                if theme::link(ui, if open { "Hide what is new" } else { "What is new" }) {
                    self.update_changes_open = !open;
                }
                if self.update_changes_open {
                    egui::Frame::new().fill(theme::BG).corner_radius(egui::CornerRadius::same(8)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ScrollArea::vertical().max_height(130.0).id_salt("changes").show(ui, |ui| {
                            ui.label(RichText::new(offer.changes.clone()).size(13.5));
                        });
                    });
                }
            }
            let ready = self.backup_choice(ui, offer.finish);
            self.backup_error_note(ui);
            go = theme::button_enabled(ui, if offer.finish { "Finish update" } else { "Update now" }, !running && ready);
            if running {
                ui.label(theme::muted("Close the game first."));
            }
        });
        if go {
            self.begin_update(&offer.to, offer.finish);
        }
    }

    // ------------------------------------------------------------------ pick the disc (first install, repair)

    fn show_pick(&mut self, ui: &mut Ui, g: &'static GameDef) {
        let repair = self.view == View::Repair;
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            self.hero(ui, g, |this, ui| {
                ui.label(theme::bold(if repair { "Repair / rebuild" } else { "Set up Ratchet & Clank" }, 22.0));
                let intro = if repair {
                    "This makes the game code again from your disc and rebuilds the game (about 15 minutes). Pick the .iso image of your Ratchet & Clank disc (PAL / Europe or USA, v1.00)."
                } else {
                    "Pick the .iso image of your own Ratchet & Clank disc (PAL / Europe or USA, v1.00). The game is built from it on this PC."
                };
                ui.label(RichText::new(intro).color(theme::TEXT).size(14.5));
                this.disc_picker(ui);
                if !repair {
                    ui.add_space(2.0);
                    ui.checkbox(&mut this.shortcut_wanted, "Create a desktop shortcut");
                }
                ui.label(theme::muted("Needs about 5 GB of free disk space and an internet connection (once)."));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let ok = matches!(this.disc_state, DiscState::Ok(_)) && this.disc_path.is_some();
                    if theme::primary_button(ui, if repair { "Repair" } else { "Install" }, Vec2::new(170.0, 52.0), ok) {
                        this.start_install();
                    }
                    if repair && theme::button(ui, "Back") {
                        this.view = View::Home;
                    }
                });
            });
        });
    }

    fn disc_picker(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 110.0).max(120.0);
            let r = ui.add(egui::TextEdit::singleline(&mut self.disc_text).hint_text("Path of your .iso file").desired_width(w).margin(egui::Margin::symmetric(10, 8)));
            if r.changed() {
                self.disc_text_changed();
            }
            if theme::button_enabled(ui, "Browse...", self.picker.is_none()) {
                self.browse(Purpose::Iso);
            }
        });
        match self.disc_state.clone() {
            DiscState::Empty => {
                ui.label(theme::muted("You can also drag the .iso file into this window."));
            }
            DiscState::Checking(p) => {
                theme::progress(ui, p as f32 / 100.0, 6.0, theme::BLUE);
                ui.label(theme::muted(format!("Checking the disc image ... {p}%")));
            }
            DiscState::Ok(d) => {
                ui.label(RichText::new(format!("Disc OK: Ratchet & Clank, {}.", d.name)).color(theme::GREEN).size(14.5));
            }
            DiscState::Bad(m) => {
                ui.label(RichText::new(m).color(theme::RED).size(14.0));
            }
        }
        ui.label(theme::muted("Your disc image is only read. Nothing is uploaded."));
    }

    pub fn run_title(&self) -> String {
        match &self.run_kind {
            RunKind::Install => "Installing Ratchet & Clank".to_string(),
            RunKind::Repair => "Repairing Ratchet & Clank".to_string(),
            RunKind::Update { to, finish } => {
                if *finish {
                    format!("Finishing the update to v{to}")
                } else {
                    format!("Updating to v{to}")
                }
            }
        }
    }
}
