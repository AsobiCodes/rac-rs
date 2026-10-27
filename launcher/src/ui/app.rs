//! The eframe application: sidebar + page routing, drag-and-drop, toast, "setup still running" confirmation, window size on exit.
use super::state::{App, Sel, Tab, View};
use super::theme;
use egui::{Align2, Margin, Ui, Vec2};
use std::time::Duration;

pub struct LauncherApp {
    pub app: App,
    size: Vec2,
}

impl LauncherApp {
    pub fn new(app: App) -> LauncherApp {
        LauncherApp { size: Vec2::new(app.cfg.width, app.cfg.height), app }
    }

    /// Draws the whole window into `ui` (the root ui of the frame). Also used by the snapshot tests.
    pub fn draw(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.app.profile_ui.anchor = None;
        let side = egui::Frame::new().fill(theme::SIDEBAR).inner_margin(Margin::symmetric(14, 16));
        egui::Panel::left("side").exact_size(236.0).resizable(false).frame(side).show(ui, |ui| self.app.show_sidebar(ui));
        let main = egui::Frame::new().fill(theme::BG).inner_margin(Margin::symmetric(30, 24));
        egui::CentralPanel::default().frame(main).show(ui, |ui| {
            self.app.launcher_banner(ui);
            match self.app.sel {
            Sel::Launcher => self.app.show_launcher_page(ui),
            Sel::Library => self.app.show_library(ui),
            Sel::Game(id) => match crate::games::game_by_id(id) {
                Some(g) if !g.supported => self.app.show_coming_soon(ui, g),
                _ => self.app.show_game_page(ui),
            },
            }
        });
        self.app.profile_popup(&ctx);
        self.handle_drops(&ctx);
        self.show_toast(&ctx);
        self.show_quit_confirm(&ctx);
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        for f in dropped {
            let p = f.path().to_path_buf();
            let is_iso = p.extension().map(|e| e.eq_ignore_ascii_case("iso")).unwrap_or(false);
            if self.app.sel == Sel::Game("rac1") && self.app.tab == Tab::Mods && self.app.info.installed && self.app.view == View::Home {
                self.app.mods_inspect(&p);
            } else if is_iso && matches!(self.app.view, View::Home | View::Repair) && !self.app.busy() {
                self.app.set_disc_path(&p.to_string_lossy());
            }
        }
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if hovering {
            let rect = ctx.content_rect();
            let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop")));
            p.rect_filled(rect, 0, egui::Color32::from_rgba_unmultiplied(0, 0, 0, 150));
            p.text(rect.center(), Align2::CENTER_CENTER, "Drop it here", egui::FontId::new(30.0, theme::bold_family()), theme::ORANGE);
        }
    }

    fn show_toast(&mut self, ctx: &egui::Context) {
        let Some(t) = self.app.toast.as_ref() else { return };
        let (text, err) = (t.text.clone(), t.error);
        egui::Area::new(egui::Id::unique("toast")).anchor(Align2::RIGHT_BOTTOM, Vec2::new(-20.0, -20.0)).order(egui::Order::Tooltip).show(ctx, |ui| {
            let edge = if err { theme::RED } else { theme::GREEN };
            egui::Frame::new()
                .fill(theme::CARD_HI)
                .stroke(egui::Stroke::new(1.5, edge))
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    ui.set_max_width(420.0);
                    ui.label(egui::RichText::new(text).size(14.0).color(theme::TEXT));
                });
        });
    }

    fn show_quit_confirm(&mut self, ctx: &egui::Context) {
        if !self.app.confirm_quit {
            return;
        }
        let mut stay = false;
        let mut quit = false;
        egui::Modal::new(egui::Id::unique("quit")).show(ctx, |ui| {
            ui.set_width(380.0);
            let backing = self.app.backup_running();
            ui.label(theme::bold(if backing { "A backup is still running" } else { "Setup is still running" }, 20.0));
            ui.add_space(6.0);
            ui.label(theme::muted(if backing {
                "If you close the launcher now, the backup is stopped and removed. The update has not started and nothing was changed."
            } else {
                "If you close the launcher now, the setup stops. You can start it again later and it continues."
            }));
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if theme::button(ui, "Keep running") {
                    stay = true;
                }
                if theme::button(ui, "Stop and close") {
                    quit = true;
                }
            });
        });
        if stay {
            self.app.confirm_quit = false;
        }
        if quit {
            self.app.cancel_run();
            self.app.confirm_quit = false;
            self.app.quit_now = true;
        }
    }

    /// Reads the command line. `--root DIR` = "use this install" (tests, Test_Launcher.cmd); otherwise the library decides.
    pub fn open_args() -> StartInfo {
        let a = super::state::parse_args(std::env::args().skip(1));
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let appdata = crate::platform::data_base();
        let data_dir = match &a.root {
            Some(r) => r.clone(),
            None => crate::library::data_dir(&exe_dir, appdata.as_deref()).0,
        };
        let base = a.update_base.or_else(|| std::env::var("RAC_LAUNCHER_UPDATE_BASE").ok().filter(|s| !s.is_empty()));
        StartInfo { data_dir, exe_dir, root: a.root, base, restarted: a.restarted }
    }

    /// The app for the command line: the library of `data_dir` (the launcher's own folder is added when it sits in a game folder).
    pub fn build_app(info: &StartInfo) -> App {
        let mut app = Self::build_app_inner(info);
        app.start_self_check();
        app
    }

    fn build_app_inner(info: &StartInfo) -> App {
        match &info.root {
            Some(r) => App::new(r.clone(), info.base.clone()),
            None => {
                let mut lib = crate::library::Library::load(&info.data_dir);
                if lib.auto_add(&info.exe_dir).is_some() {
                    let _ = lib.save();
                }
                App::with_library(info.data_dir.clone(), lib, None, info.base.clone())
            }
        }
    }
}

/// What the command line and the exe's place say.
pub struct StartInfo {
    pub data_dir: std::path::PathBuf,
    pub exe_dir: std::path::PathBuf,
    /// `--root`.
    pub root: Option<std::path::PathBuf>,
    pub base: Option<String>,
    pub restarted: bool,
}

impl eframe::App for LauncherApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.viewport().close_requested()) && !self.app.quit_now {
            if self.app.busy() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.app.confirm_quit = true;
            }
        }
        if let Some(r) = ctx.input(|i| i.viewport().inner_rect) {
            if r.width() > 100.0 && r.height() > 100.0 {
                self.size = r.size();
            }
        }
        let active = self.app.tick(&ctx);
        self.draw(ui);
        if active {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if self.app.quit_now {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.app.cfg.width = self.size.x;
        self.app.cfg.height = self.size.y;
        self.app.save_cfg();
        if self.app.busy() {
            self.app.cancel_run();
        }
    }
}
