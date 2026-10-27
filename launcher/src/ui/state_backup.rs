//! Manual backups: a playable copy of the open install next to it, and the Library actions on a
//! backup ("Delete backup" through the trash, "Play").
//! The copy itself is `backup.rs`; the drawing is `page_backup.rs`.
use super::state::{App, Sel, View};
use crate::backup;
use crate::platform;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A backup that is being made right now.
pub struct RunningBackup {
    pub state: Arc<backup::State>,
    /// The version being backed up.
    pub version: String,
    pub cancelling: bool,
}

#[derive(Default)]
pub struct BackupUi {
    pub job: Option<RunningBackup>,
    /// Packs the last backup had to leave out (said once when it is done).
    pub left_out: Vec<String>,
    /// Tests only: pretend this many bytes are free on the drive.
    pub free_override: Option<u64>,
}

impl App {
    /// The open install is a backup (never rebuilt or backed up again).
    pub fn is_backup(&self) -> bool {
        self.cur_id.as_deref().and_then(|id| self.library.get(id)).map(|e| e.backup).unwrap_or(false)
    }

    /// A backup is being made.
    pub fn backup_running(&self) -> bool {
        self.backup.job.is_some()
    }

    /// "Back up this install": copy the built game, saves, settings and packs next to the install.
    pub fn begin_backup(&mut self) {
        if self.backup.job.is_some() {
            return;
        }
        if self.is_backup() {
            self.toast("This is already a backup. Open your main install to back it up.", true);
            return;
        }
        if !self.info.installed {
            self.toast("The game is not built here, so there is nothing to back up.", true);
            return;
        }
        if self.game_running() {
            self.toast("Close the game first, then back it up.", true);
            return;
        }
        let version = self.info.local_version.clone();
        let state = backup::start_fresh(self.root.clone(), self.rac1(), version.clone());
        self.backup.job = Some(RunningBackup { state, version, cancelling: false });
        self.backup.left_out.clear();
        self.failure = None;
        self.view = View::Backup;
    }

    pub fn cancel_backup(&mut self) {
        if let Some(j) = self.backup.job.as_mut() {
            j.state.cancel();
            j.cancelling = true;
        }
    }

    /// Cancels a running backup and waits (a few seconds at most) until its folder is removed. Used when the launcher closes.
    pub fn stop_backup(&mut self) {
        let Some(j) = self.backup.job.take() else { return };
        j.state.cancel();
        let end = Instant::now() + Duration::from_secs(15);
        while !j.state.finished() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(20));
        }
        self.view = View::Home;
    }

    /// Called every frame: when the copy is over, list the backup in the Library.
    pub fn poll_backup(&mut self) {
        let Some(res) = self.backup.job.as_ref().and_then(|j| j.state.take_result()) else { return };
        let Some(job) = self.backup.job.take() else { return };
        self.view = View::Home;
        match res {
            Ok(()) => {
                let Some(plan) = job.state.plan() else { return };
                let g = self.rac1();
                let e = self.library.add_backup(g, &plan.dest, &plan.name);
                self.save_library();
                self.backup.left_out = job.state.left_out_names();
                let mut m = format!("Backup made: {}. It is in your Library.", e.name);
                if !self.backup.left_out.is_empty() {
                    m.push_str(&format!(" The texture pack(s) {} were left out (this drive cannot share files).", self.backup.left_out.join(", ")));
                }
                self.toast(m, false);
            }
            Err(m) if m == backup::CANCELLED => {
                self.toast("The backup was cancelled and removed. Nothing was changed.", false);
            }
            Err(m) => {
                self.toast(format!("The backup could not be made. Nothing was changed.\n{m}"), true);
            }
        }
    }

    /// "Delete backup": the folder goes to the trash (hard-linked packs keep living in the main install), then the entry goes.
    pub fn delete_backup(&mut self, id: &str) {
        let Some(e) = self.library.get(id).cloned().filter(|e| e.backup) else { return };
        let g = self.rac1();
        if (self.cur_id.as_deref() == Some(id) && (self.busy() || self.game_running())) || platform::exe_in_use(&g.exe_path(&e.path())) {
            self.toast("Close the game first, then delete the backup.", true);
            return;
        }
        if e.path().is_dir() {
            if let Err(err) = platform::recycle(&e.path()) {
                self.toast(format!("The backup could not be moved to the trash: {err}"), true);
                return;
            }
        }
        self.lib_ui.confirm_delete = None;
        self.remove_install(id);
        if self.library.entries.is_empty() {
            self.sel = Sel::Library;
        }
        self.toast("The backup was moved to the trash.", false);
    }
}
