//! Drawing for the network flows: the game download card (Library page) and the launcher-update banner (top of every page).
use super::state::App;
use super::theme;
use crate::game_dl::{STAGE_DOWNLOAD, STAGE_UNPACK};
use crate::library::size_text;
use crate::releases;
use crate::selfupdate::{SelfState, OWN_VERSION};
use egui::{RichText, ScrollArea, Ui};
use std::sync::atomic::Ordering;

/// "412 MB of 960 MB - 8.4 MB/s".
pub fn transfer_text(done: u64, total: u64, speed: f64) -> String {
    let mut t = if total > 0 { format!("{} of {}", size_text(done), size_text(total)) } else { size_text(done) };
    if speed > 1.0 {
        t.push_str(&format!(" - {}/s", size_text(speed as u64)));
    }
    t
}

impl App {
    /// The card of a running or failed game download. Nothing when there is none.
    pub fn download_card(&mut self, ui: &mut Ui) {
        if let Some(d) = &self.net.game_dl {
            let stage = d.stage.load(Ordering::Relaxed);
            let (done, total) = (d.transfer.done.load(Ordering::Relaxed), d.transfer.total.load(Ordering::Relaxed));
            let ver = d.version.lock().map(|v| v.clone()).unwrap_or_default();
            let title = match stage {
                STAGE_DOWNLOAD => format!("Downloading RAC-RS {ver}"),
                STAGE_UNPACK => format!("Unpacking RAC-RS {ver}"),
                _ => "Looking for the latest release...".to_string(),
            };
            let frac = if total > 0 && stage >= STAGE_DOWNLOAD { (done as f32 / total as f32).clamp(0.0, 1.0) } else { 0.0 };
            let line = if stage >= STAGE_DOWNLOAD { transfer_text(done, total, releases::speed(done, d.started)) } else { "Contacting GitHub".to_string() };
            let dest = d.dest.display().to_string();
            let mut cancel = false;
            theme::card_colored(ui, theme::BLUE, |ui| {
                ui.set_width(ui.available_width());
                ui.label(theme::bold(title, 17.0));
                ui.label(theme::muted(format!("Into {dest}")));
                ui.add_space(4.0);
                theme::progress(ui, frac, 12.0, theme::BLUE);
                ui.label(RichText::new(line).size(14.0));
                cancel = theme::button(ui, "Cancel download");
            });
            ui.add_space(8.0);
            if cancel {
                self.cancel_download();
            }
        } else if let Some(msg) = self.net.dl_error.clone() {
            let mut dismiss = false;
            theme::card_colored(ui, theme::RED, |ui| {
                ui.set_width(ui.available_width());
                ui.label(theme::bold("The download did not finish", 17.0));
                ui.label(RichText::new(msg).color(theme::TEXT).size(14.5));
                dismiss = theme::button(ui, "OK");
            });
            ui.add_space(8.0);
            if dismiss {
                self.net.dl_error = None;
            }
        }
    }

    /// "A new launcher is available" with the release notes and an Update button; while it downloads, a progress bar.
    pub fn launcher_banner(&mut self, ui: &mut Ui) {
        let SelfState::Available(upd) = self.self_state() else { return };
        let mut go = false;
        let mut toggle = false;
        let progress = self.net.self_apply.as_ref().map(|a| (a.transfer.done.load(Ordering::Relaxed), a.transfer.total.load(Ordering::Relaxed), a.started));
        let err = self.net.self_error.clone();
        let open = self.net.notes_open;
        theme::card_colored(ui, theme::BLUE, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::chip(ui, "Launcher update", theme::BLUE);
                ui.label(theme::bold(format!("Launcher {} is available (you have {OWN_VERSION})", upd.version), 16.0));
            });
            if !upd.notes.trim().is_empty() {
                if theme::link(ui, if open { "Hide what is new" } else { "What is new" }) {
                    toggle = true;
                }
                if open {
                    egui::Frame::new().fill(theme::BG).corner_radius(egui::CornerRadius::same(8)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ScrollArea::vertical().max_height(120.0).id_salt("launcher_notes").show(ui, |ui| {
                            ui.label(RichText::new(upd.notes.clone()).size(13.5));
                        });
                    });
                }
            }
            match progress {
                Some((done, total, since)) => {
                    let frac = if total > 0 { (done as f32 / total as f32).clamp(0.0, 1.0) } else { 0.0 };
                    theme::progress(ui, frac, 10.0, theme::BLUE);
                    ui.label(theme::muted(format!("Downloading... {}", transfer_text(done, total, releases::speed(done, since)))));
                }
                None => {
                    go = theme::button(ui, "Update launcher");
                    ui.label(theme::muted("The launcher restarts by itself. Your games and settings are not touched."));
                }
            }
            if let Some(e) = err {
                ui.label(RichText::new(e).color(theme::RED).size(14.0));
            }
        });
        ui.add_space(8.0);
        if toggle {
            self.net.notes_open = !open;
        }
        if go {
            self.apply_launcher_update();
        }
    }
}

impl App {
    /// The version picker of the "New install" form: default = newest, older versions with their date, "Show test versions" tick.
    /// Older versions are installed side by side as their own entry and stay on that version (no update nag).
    pub fn version_picker(&mut self, ui: &mut Ui) {
        use crate::ui::state_net::VersionList;
        let list = self.installable_versions();
        let picked = self.picked_version();
        let newest = crate::game_dl::default_pick(&list).map(|i| i.version.clone());
        match self.version_list() {
            VersionList::Idle | VersionList::Loading => {
                ui.label(theme::muted("Looking up the available versions..."));
            }
            VersionList::Failed(m) => {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(m).color(theme::RED).size(14.0));
                    if theme::button(ui, "Try again") {
                        self.load_versions(true);
                    }
                });
            }
            VersionList::Ready(_) if list.is_empty() => {
                ui.label(theme::muted("No downloadable version was found."));
            }
            VersionList::Ready(_) => {
                let text_of = |i: &crate::game_dl::Installable| {
                    let mut t = format!("Version {}", i.version);
                    if Some(&i.version) == newest.as_ref() {
                        t.push_str(" (newest)");
                    }
                    if i.prerelease {
                        t.push_str(" (test)");
                    }
                    if !i.date.is_empty() {
                        t.push_str(&format!(" - {}", i.date));
                    }
                    t
                };
                let current = list.iter().find(|i| Some(&i.version) == picked.as_ref()).map(text_of).unwrap_or_default();
                let mut chosen: Option<String> = None;
                ui.horizontal(|ui| {
                    ui.label(theme::muted("Game version"));
                    let combo = egui::ComboBox::from_id_salt("game_version").selected_text(current).width(300.0).height(340.0).show_ui(ui, |ui| {
                        for i in &list {
                            if ui.selectable_label(Some(&i.version) == picked.as_ref(), text_of(i)).clicked() {
                                chosen = Some(i.version.clone());
                            }
                        }
                    });
                    combo.response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Game version"));
                });
                let mut show = self.net.show_test;
                if ui.checkbox(&mut show, "Show test versions").changed() {
                    self.net.show_test = show;
                }
                if let Some(c) = chosen {
                    self.net.pick = Some(c);
                }
                if picked.is_some() && picked != newest {
                    ui.label(theme::muted("An older version is installed next to your other installs and stays on that version. Your current install is not changed."));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_line() {
        assert_eq!(transfer_text(0, 0, 0.0), "0 KB");
        let t = transfer_text(5 * 1024 * 1024, 10 * 1024 * 1024, 2.0 * 1024.0 * 1024.0);
        assert_eq!(t, "5.0 MB of 10.0 MB - 2.0 MB/s");
    }
}
