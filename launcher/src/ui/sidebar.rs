//! The left column: brand, the games, and the launcher's own page; plus the "coming soon" page and the launcher page.
use super::state::{App, Sel};
use super::theme;
use crate::games::{self, GameDef};
use crate::library::LibEntry;
use crate::update::UpdateState;
use egui::{Align2, Color32, CornerRadius, FontId, Rect, Sense, Ui, Vec2};

/// Version of the launcher program itself.
pub const LAUNCHER_VERSION: &str = env!("CARGO_PKG_VERSION");

fn folder_tail(folder: &str) -> String {
    std::path::Path::new(folder).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| folder.to_string())
}

fn ellipsis(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(max.saturating_sub(3)).collect::<String>())
    }
}

/// The status line of an install that is not the open one.
fn entry_line(e: &LibEntry) -> (String, Option<Color32>) {
    let dir = e.path();
    let g = games::game_by_id(&e.game).unwrap_or(&games::GAMES[0]);
    if !dir.is_dir() {
        return ("Folder not found".into(), Some(theme::RED));
    }
    if !games::is_installed(&dir, g) {
        return ("Not set up yet".into(), Some(theme::ORANGE));
    }
    if e.backup {
        return (format!("{} backup", g.title), Some(theme::BLUE));
    }
    (format!("Version {}", games::local_version(&dir)), Some(theme::GREEN))
}

/// One row of the sidebar: a title with a small line below. Returns true when clicked.
fn nav_item(ui: &mut Ui, selected: bool, title: &str, sub: &str, dot: Option<Color32>) -> bool {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 52.0), Sense::click());
    let p = ui.painter();
    let r = CornerRadius::same(10);
    if selected {
        p.rect_filled(rect, r, theme::CARD_HI);
        p.rect_filled(Rect::from_min_size(rect.min + Vec2::new(0.0, 10.0), Vec2::new(4.0, rect.height() - 20.0)), CornerRadius::same(2), theme::ORANGE);
    } else if resp.hovered() {
        p.rect_filled(rect, r, theme::CARD);
    }
    let col = if selected { theme::TEXT } else { Color32::from_rgb(0xC4, 0xCB, 0xD8) };
    p.text(rect.left_top() + Vec2::new(18.0, 11.0), Align2::LEFT_TOP, title, FontId::new(15.5, theme::bold_family()), col);
    p.text(rect.left_top() + Vec2::new(18.0, 31.0), Align2::LEFT_TOP, sub, FontId::proportional(12.5), theme::MUTED);
    if let Some(c) = dot {
        p.circle_filled(egui::pos2(rect.right() - 16.0, rect.center().y), 4.5, c);
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.clicked()
}

/// A small "Support the project" link with a painted heart (a Ko-fi page). It opens the browser only on a click.
pub fn support_link(ui: &mut Ui) {
    let font = FontId::proportional(13.0);
    let label = "Support the project";
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, theme::MUTED);
    let size = Vec2::new(14.0 + 7.0 + galley.size().x, 18.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Link, true, label));
    let hot = resp.hovered() || resp.has_focus();
    let p = ui.painter();
    let o = rect.left_top() + Vec2::new(0.0, (size.y - 12.0) / 2.0);
    let heart = if hot { Color32::from_rgb(0xFF, 0x6B, 0x81) } else { Color32::from_rgb(0xE0, 0x5A, 0x6E) };
    p.circle_filled(o + Vec2::new(4.3, 4.2), 3.7, heart);
    p.circle_filled(o + Vec2::new(9.7, 4.2), 3.7, heart);
    p.add(egui::Shape::convex_polygon(vec![o + Vec2::new(0.8, 5.6), o + Vec2::new(13.2, 5.6), o + Vec2::new(7.0, 12.0)], heart, egui::Stroke::NONE));
    let col = if hot { theme::TEXT } else { theme::MUTED };
    let text_pos = rect.left_center() + Vec2::new(21.0, 0.0);
    p.galley(text_pos - Vec2::new(0.0, galley.size().y / 2.0), galley, col);
    if hot {
        let y = rect.bottom() - 1.0;
        p.line_segment([egui::pos2(rect.left() + 21.0, y), egui::pos2(rect.right(), y)], egui::Stroke::new(1.0, col));
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if resp.clicked() {
        crate::platform::open_url(crate::play::SUPPORT_URL);
    }
}

impl App {
    fn game_line(&self, g: &GameDef) -> (String, Option<Color32>) {
        if !g.supported {
            return ("Coming soon".into(), None);
        }
        if self.view == super::state::View::Run {
            return ("Setting up...".into(), Some(theme::BLUE));
        }
        if self.view == super::state::View::Backup {
            return ("Backing up...".into(), Some(theme::BLUE));
        }
        if !self.info.installed {
            return ("Not installed".into(), Some(theme::MUTED));
        }
        if self.game_running() {
            return ("Running".into(), Some(theme::GREEN));
        }
        let upd = matches!(self.update_state(), UpdateState::Available(_)) && !self.is_pinned() && !self.is_backup();
        let v = if self.info.local_version.is_empty() { "Installed".to_string() } else { format!("Version {}", self.info.local_version) };
        (v, Some(if upd { theme::ORANGE } else { theme::GREEN }))
    }

    pub fn show_sidebar(&mut self, ui: &mut Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.label(theme::bold("RAC", 25.0).color(theme::ORANGE));
            ui.label(theme::bold("-RS", 25.0).color(theme::BLUE));
        });
        ui.label(theme::muted("Launcher"));
        ui.add_space(22.0);
        ui.label(egui::RichText::new("GAMES").size(11.5).color(theme::MUTED).strong());
        ui.add_space(4.0);
        for g in games::visible_games() {
            if !g.supported {
                let (line, dot) = self.game_line(g);
                if nav_item(ui, self.sel == Sel::Game(g.id), g.title, &line, dot) {
                    self.sel = Sel::Game(g.id);
                }
                ui.add_space(2.0);
                continue;
            }
            let installs: Vec<LibEntry> = self.library.entries.iter().filter(|e| e.game == g.id).cloned().collect();
            if installs.is_empty() {
                if nav_item(ui, false, g.title, "Not installed", Some(theme::MUTED)) {
                    self.sel = Sel::Library;
                }
                ui.add_space(2.0);
            }
            for e in &installs {
                let current = self.cur_id.as_deref() == Some(e.id.as_str());
                let (mut line, dot) = if current && !e.backup { self.game_line(g) } else { entry_line(e) };
                if installs.len() > 1 && !e.backup {
                    line = format!("{line} - {}", folder_tail(&e.folder));
                }
                let selected = current && self.sel == Sel::Game(g.id);
                // a backup's list name is long ("... v0.3.1 (backup)"); the line below says which game
                let title = if e.backup { format!("Backup v{}", games::local_version(&e.path())) } else { e.title(g) };
                if nav_item(ui, selected, &ellipsis(&title, 24), &ellipsis(&line, 27), dot) {
                    self.select_install(&e.id);
                }
                ui.add_space(2.0);
            }
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.label(theme::muted(format!("Launcher {LAUNCHER_VERSION}")));
            ui.add_space(4.0);
            support_link(ui);
            ui.add_space(6.0);
            if nav_item(ui, self.sel == Sel::Launcher, "Launcher", "Folder, shortcut, window", None) {
                self.sel = Sel::Launcher;
            }
            ui.add_space(2.0);
            if nav_item(ui, self.sel == Sel::Library, "Library", &format!("{} install(s) - add or remove", self.library.entries.len()), None) {
                self.sel = Sel::Library;
            }
        });
    }

    /// The page of a game that is announced but not playable yet.
    pub fn show_coming_soon(&mut self, ui: &mut Ui, g: &'static GameDef) {
        ui.label(theme::bold(g.title, 26.0));
        ui.add_space(24.0);
        theme::card_colored(ui, theme::BLUE, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.add_space(26.0);
                theme::chip(ui, "Coming soon", theme::BLUE);
                ui.add_space(14.0);
                ui.label(theme::bold(format!("{} is on its way", g.title), 24.0));
                ui.add_space(8.0);
                ui.label(theme::muted("It will appear here as soon as it is ready, with the same one-click install as Ratchet & Clank."));
                ui.label(theme::muted("Your Ratchet & Clank install is not touched. Nothing to do for now."));
                ui.add_space(26.0);
            });
        });
    }

    /// The launcher's own settings.
    pub fn show_launcher_page(&mut self, ui: &mut Ui) {
        ui.label(theme::bold("Launcher", 26.0));
        ui.label(theme::muted("Settings of this window, not of the game."));
        ui.add_space(14.0);
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(theme::bold("Minimize while playing", 15.5));
                    ui.label(theme::muted("Hides this window while the game runs and brings it back when you quit the game."));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::switch(ui, self.cfg.minimize_while_playing, true, "Minimize while playing") {
                        self.cfg.minimize_while_playing = !self.cfg.minimize_while_playing;
                        self.save_cfg();
                    }
                });
            });
        });
        ui.add_space(12.0);
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(theme::bold("This install", 15.5));
            ui.add_space(4.0);
            ui.label(theme::muted(format!("Folder: {}", if self.has_install() { self.root.display().to_string() } else { "-".to_string() })));
            ui.label(theme::muted(format!("Game version: {}", if self.info.local_version.is_empty() { "-" } else { &self.info.local_version })));
            ui.label(theme::muted(format!("Launcher version: {LAUNCHER_VERSION}")));
            ui.label(theme::muted(format!("Library file: {}", self.library.file.display())));
            ui.label(theme::muted(crate::play::SAVES_HINT));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if theme::button(ui, "Open game folder") {
                    crate::platform::open_path(&self.root);
                }
                if self.has_install() && theme::button(ui, "Open saves folder") {
                    if let Err(e) = crate::play::open_saves(&self.root) {
                        self.toast(format!("The saves folder could not be opened: {e}"), true);
                    }
                }
                if self.has_install() && theme::button(ui, "Open log folder") {
                    if let Err(e) = crate::play::open_logs(&self.root) {
                        self.toast(format!("The log folder could not be opened: {e}"), true);
                    }
                }
                if theme::button(ui, "Create desktop shortcut") {
                    match self.create_shortcut() {
                        Ok(_) => self.toast("Desktop shortcut created.", false),
                        Err(e) => self.toast(format!("Could not create the shortcut: {e}"), true),
                    }
                }
            });
        });
    }
}
