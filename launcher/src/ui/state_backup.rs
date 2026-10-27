//! The flows behind "Back up your current version first": plan (space, what is shared), run the copy on a worker, list the backup in the
//! library, start the update; and the Library actions on a backup ("Delete backup" through the Recycle Bin, "Play the backup").
//! The copy itself is `backup.rs`; the drawing is `page_backup.rs`.
use super::state::{App, Sel, View};
use crate::backup::{self, Plan};
use crate::library;
use crate::platform;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The plan for the open install, made on a thread (it walks the player folder and the mods).
#[derive(Clone, Default)]
pub enum PlanState {
    #[default]
    Idle,
    Working,
    Ready(Arc<Plan>),
    Failed(String),
}

/// A backup that is being made right now.
pub struct RunningBackup {
    pub state: Arc<backup::State>,
    /// The update that starts when the backup is done.
    pub to: String,
    pub finish: bool,
    /// The version being backed up.
    pub version: String,
    pub cancelling: bool,
}

#[derive(Default)]
pub struct BackupUi {
    pub plan: Arc<Mutex<PlanState>>,
    /// (install folder, version) the plan was made for.
    pub key: Option<(PathBuf, String)>,
    pub job: Option<RunningBackup>,
    /// Why the last backup failed (shown in the update card).
    pub error: Option<String>,
    /// Library id of the backup made for the update that is running or just failed.
    pub made: Option<String>,
    /// Packs the last backup had to leave out (told once on the done screen).
    pub left_out: Vec<String>,
    /// Tests only: pretend this many bytes are free on the drive.
    pub free_override: Option<u64>,
}

/// What the update card shows about the backup choice.
#[derive(Clone)]
pub enum Verdict {
    Working,
    Unavailable(String),
    Ready { plan: Arc<Plan>, free: Option<u64>, short: Option<String> },
}

impl App {
    /// The open install is a backup (never updated, never repaired).
    pub fn is_backup(&self) -> bool {
        self.cur_id.as_deref().and_then(|id| self.library.get(id)).map(|e| e.backup).unwrap_or(false)
    }

    /// A backup is being made.
    pub fn backup_running(&self) -> bool {
        self.backup.job.is_some()
    }

    /// Starts reading the install for the plan when it was not read for this folder and version yet; says what is known now.
    pub fn backup_verdict(&mut self) -> Verdict {
        let key = (self.root.clone(), self.info.local_version.clone());
        if self.backup.key.as_ref() != Some(&key) {
            self.backup.key = Some(key.clone());
            if let Ok(mut g) = self.backup.plan.lock() {
                *g = PlanState::Working;
            }
            let (slot, game) = (self.backup.plan.clone(), self.rac1());
            std::thread::spawn(move || {
                let r = match backup::plan(&key.0, game, &key.1) {
                    Ok(p) => PlanState::Ready(Arc::new(p)),
                    Err(m) => PlanState::Failed(m),
                };
                if let Ok(mut g) = slot.lock() {
                    *g = r;
                }
            });
        }
        let st = self.backup.plan.lock().map(|g| g.clone()).unwrap_or_default();
        match st {
            PlanState::Idle | PlanState::Working => Verdict::Working,
            PlanState::Failed(m) => Verdict::Unavailable(m),
            PlanState::Ready(plan) => {
                let free = self.backup.free_override.or_else(|| plan.dest.parent().and_then(library::free_space));
                let short = plan.space_check(free).err();
                Verdict::Ready { plan, free, short }
            }
        }
    }

    /// Forget the plan (files changed: a check for updates, a new import ...).
    pub fn backup_forget_plan(&mut self) {
        self.backup.key = None;
    }

    /// The tick is on AND the backup is possible (the plan is read, there is room).
    pub fn backup_wanted(&mut self, finish: bool) -> bool {
        !finish && self.cfg.backup_before_update && matches!(self.backup_verdict(), Verdict::Ready { short: None, .. })
    }

    /// The place every "Update" button calls: makes the backup first when it is wanted, then the update.
    pub fn begin_update(&mut self, to: &str, finish: bool) {
        self.backup.error = None;
        self.backup.made = None;
        self.backup.left_out.clear();
        if self.is_backup() {
            self.toast("A backup is never updated. Open your main install to update.", true);
            return;
        }
        if !finish && self.update_blocker().is_none() && self.backup_wanted(finish) {
            self.start_backup(to, finish);
        } else {
            self.start_update(to, finish);
        }
    }

    fn start_backup(&mut self, to: &str, finish: bool) {
        let version = self.info.local_version.clone();
        let state = backup::start_fresh(self.root.clone(), self.rac1(), version.clone());
        self.backup.job = Some(RunningBackup { state, to: to.to_string(), finish, version, cancelling: false });
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

    /// Called every frame: when the copy is over, list the backup and start the update (or tell why not).
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
                self.backup.made = Some(e.id);
                self.backup.left_out = job.state.left_out_names();
                self.start_update(&job.to, job.finish);
            }
            Err(m) if m == backup::CANCELLED => {
                self.toast("The backup was cancelled and removed. The update was not started and nothing was changed.", false);
            }
            Err(m) => {
                self.backup.error = Some(format!("The backup could not be made, so the update was not started. Nothing was changed.\n{m}"));
                self.toast("The backup could not be made. The update was not started.", true);
            }
        }
    }

    /// The backup made for the update that just failed: (library id, its version). None = no backup or it is gone.
    pub fn backup_for_failure(&self) -> Option<(String, String)> {
        let id = self.backup.made.clone()?;
        let e = self.library.get(&id).filter(|e| e.backup)?;
        e.path().is_dir().then(|| (id, crate::games::local_version(&e.path())))
    }

    /// "Play the backup (vOLD)" on the error screen of a failed update.
    pub fn play_the_backup(&mut self, ctx: &egui::Context) {
        let Some((id, _)) = self.backup_for_failure() else { return };
        self.failure = None;
        if self.select_install(&id) {
            self.play(ctx);
        }
    }

    /// "Delete backup": the folder goes to the Recycle Bin (hard-linked packs keep living in the main install), then the entry goes.
    pub fn delete_backup(&mut self, id: &str) {
        let Some(e) = self.library.get(id).cloned().filter(|e| e.backup) else { return };
        let g = self.rac1();
        if (self.cur_id.as_deref() == Some(id) && (self.busy() || self.game_running())) || platform::exe_in_use(&g.exe_path(&e.path())) {
            self.toast("Close the game first, then delete the backup.", true);
            return;
        }
        if e.path().is_dir() {
            if let Err(err) = platform::recycle(&e.path()) {
                self.toast(format!("The backup could not be moved to the Recycle Bin: {err}"), true);
                return;
            }
        }
        self.lib_ui.confirm_delete = None;
        self.remove_install(id);
        if self.backup.made.as_deref() == Some(id) {
            self.backup.made = None;
        }
        if self.library.entries.is_empty() {
            self.sel = Sel::Library;
        }
        self.toast("The backup was moved to the Recycle Bin.", false);
    }
}
