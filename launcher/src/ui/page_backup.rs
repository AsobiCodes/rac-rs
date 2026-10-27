//! Drawing of the backup: the choice in the update card ("Back up your current version first"), the progress card while it is copied,
//! and the lines of a backup install (Play tab note). The flows are in `state_backup.rs`, the copy in `backup.rs`.
use super::state::App;
use super::state_backup::Verdict;
use super::theme;
use crate::library::size_text;
use egui::{RichText, Ui, Vec2};

impl App {
    /// The tick with its numbers, under the update text of the card. Returns false while the plan is still being read (the update
    /// button waits for it, so a click never starts without the choice being known).
    pub fn backup_choice(&mut self, ui: &mut Ui, finish: bool) -> bool {
        if finish {
            ui.label(theme::muted("No backup is possible now: the files of the new version are already in place."));
            return true;
        }
        let version = self.info.local_version.clone();
        match self.backup_verdict() {
            Verdict::Working => {
                ui.label(theme::muted("Checking the space for a backup..."));
                false
            }
            Verdict::Unavailable(why) => {
                ui.add_enabled(false, egui::Checkbox::new(&mut false, format!("Back up your current version (v{version}) first")));
                ui.label(theme::muted(format!("A backup is not possible: {why}")));
                true
            }
            Verdict::Ready { plan, free, short } => {
                let can = short.is_none();
                let mut on = self.cfg.backup_before_update && can;
                let label = format!("Back up your current version (v{version}) first");
                let changed = ui.add_enabled(can, egui::Checkbox::new(&mut on, label)).changed();
                if changed && can {
                    self.cfg.backup_before_update = on;
                    self.save_cfg();
                }
                let free_text = free.map(|f| format!("{} free on this drive", size_text(f))).unwrap_or_else(|| "free space unknown".to_string());
                match &short {
                    Some(why) => {
                        ui.label(RichText::new(format!("{why} The update runs without a backup.")).color(theme::RED).size(13.5));
                    }
                    None => {
                        ui.label(theme::muted(format!("Needs about {} ({free_text}). The backup shows up in your Library, plays at once and keeps its own saves.", size_text(plan.copy_bytes()))));
                    }
                }
                if !plan.packs.is_empty() {
                    let (n, bytes) = (plan.packs.len(), plan.packs.iter().map(|p| p.bytes).sum::<u64>());
                    let what = if n == 1 { "texture pack".to_string() } else { format!("{n} texture packs") };
                    let line = if plan.link_ok {
                        format!("Your {what} ({}) are shared with the backup, not copied again.", size_text(bytes))
                    } else {
                        format!("Your {what} ({}) are left out: this drive cannot share files between folders. The backup plays with the original textures.", size_text(bytes))
                    };
                    ui.label(theme::muted(line));
                }
                true
            }
        }
    }

    /// The red message after a failed backup (the update did not start).
    pub fn backup_error_note(&mut self, ui: &mut Ui) {
        if let Some(m) = self.backup.error.clone() {
            ui.label(RichText::new(m).color(theme::RED).size(13.5));
        }
    }

    /// The page while the backup is copied.
    pub fn show_backup_progress(&mut self, ui: &mut Ui) {
        let Some(job) = self.backup.job.as_ref() else { return };
        let (st, version, cancelling, to) = (job.state.clone(), job.version.clone(), job.cancelling, job.to.clone());
        let mut cancel = false;
        let w = ui.available_width().min(700.0);
        ui.allocate_ui(Vec2::new(w, ui.available_height()), |ui| {
            ui.set_width(w);
            ui.add_space(10.0);
            ui.label(theme::bold(format!("Backing up version {version} first"), 22.0));
            ui.label(theme::muted(format!("Your install and your saves are only copied, not changed. Version {to} is installed when the backup is done.")));
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
                    ui.label(theme::muted("Cancel removes the partly made backup. The update is not started."));
                }
            });
        });
        if cancel {
            self.cancel_backup();
        }
    }

    /// Play tab of a backup install: what it is, and that it is never updated.
    pub fn backup_note(&mut self, ui: &mut Ui) {
        theme::card_colored(ui, theme::BLUE, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::chip(ui, "Backup", theme::BLUE);
                ui.label(theme::bold(format!("Version {} as it was before an update", self.info.local_version), 16.0));
            });
            ui.label(theme::muted("This copy never updates and has its own saves and settings, so the newer version cannot change it. Delete it in the Library when you do not need it any more."));
        });
    }
}
