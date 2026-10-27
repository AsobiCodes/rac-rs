//! Drawing of a backup: the progress card while it is copied and the lines of a backup install (Play tab).
//! The flow is in `state_backup.rs`, the copy in `backup.rs`.
use super::state::App;
use super::theme;
use crate::library::size_text;
use egui::{RichText, Ui, Vec2};

impl App {
    /// The page while the backup is copied.
    pub fn show_backup_progress(&mut self, ui: &mut Ui) {
        let Some(job) = self.backup.job.as_ref() else { return };
        let (st, version, cancelling) = (job.state.clone(), job.version.clone(), job.cancelling);
        let mut cancel = false;
        let w = ui.available_width().min(700.0);
        ui.allocate_ui(Vec2::new(w, ui.available_height()), |ui| {
            ui.set_width(w);
            ui.add_space(10.0);
            ui.label(theme::bold(format!("Backing up version {version}"), 22.0));
            ui.label(theme::muted("Your install and your saves are only copied, not changed. The backup shows up in your Library."));
            ui.add_space(8.0);
            theme::card_colored(ui, theme::BLUE, |ui| {
                ui.set_width(ui.available_width());
                let total = st.total_bytes.load(std::sync::atomic::Ordering::Relaxed);
                if total == 0 {
                    theme::progress_busy(ui, 12.0, theme::BLUE);
                    ui.label(RichText::new("Looking at your install...").size(14.5));
                } else {
                    theme::progress(ui, st.fraction(), 12.0, theme::BLUE);
                    let done = st.done_bytes.load(std::sync::atomic::Ordering::Relaxed);
                    let (fd, ft) = (st.done_files.load(std::sync::atomic::Ordering::Relaxed), st.total_files.load(std::sync::atomic::Ordering::Relaxed));
                    ui.label(RichText::new(format!("{:.0}% - {} of {} - {fd} of {ft} files", st.fraction() * 100.0, size_text(done), size_text(total))).size(14.5));
                    let cur = st.current.lock().map(|c| c.clone()).unwrap_or_default();
                    ui.label(theme::muted(cur));
                }
                ui.add_space(4.0);
                if cancelling {
                    ui.label(theme::muted("Cancelling and cleaning up..."));
                } else {
                    cancel = theme::button(ui, "Cancel");
                    ui.label(theme::muted("Cancel removes the partly made backup. The install is not touched."));
                }
            });
        });
        if cancel {
            self.cancel_backup();
        }
    }

    /// Play tab of a backup install: what it is.
    pub fn backup_note(&mut self, ui: &mut Ui) {
        theme::card_colored(ui, theme::BLUE, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::chip(ui, "Backup", theme::BLUE);
                ui.label(theme::bold(format!("Version {}", self.info.local_version), 16.0));
            });
            ui.label(theme::muted("This copy has its own saves and settings, so a newer or rebuilt version cannot change it. Delete it in the Library when you do not need it any more."));
        });
    }
}
