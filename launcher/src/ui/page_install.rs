//! While the worker runs (steps, bar, elapsed time, details log, cancel), when it failed (message, copy details, try again) and when it is done.
use super::state::{App, RunKind, View};
use super::theme;
use crate::setup_parse::{format_elapsed, Progress};
use egui::{pos2, Color32, RichText, ScrollArea, Sense, Stroke, Ui, Vec2};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StepState {
    Done,
    Active,
    Pending,
    Failed,
}

/// A 24 px round icon drawn with shapes: check, spinner, empty ring or cross.
pub fn step_icon(ui: &mut Ui, st: StepState) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
    let c = rect.center();
    let p = ui.painter();
    match st {
        StepState::Done => {
            p.circle_filled(c, 11.0, theme::GREEN);
            let s = Stroke::new(2.4, theme::BG);
            p.line_segment([pos2(c.x - 5.0, c.y + 0.5), pos2(c.x - 1.5, c.y + 4.0)], s);
            p.line_segment([pos2(c.x - 1.5, c.y + 4.0), pos2(c.x + 5.5, c.y - 3.5)], s);
        }
        StepState::Active => {
            p.circle_stroke(c, 10.5, Stroke::new(2.0, Color32::from_rgb(0x2C, 0x4A, 0x66)));
            let t = ui.input(|i| i.time) as f32 * 4.0;
            p.circle_filled(pos2(c.x + t.cos() * 10.5, c.y + t.sin() * 10.5), 3.6, theme::BLUE);
            p.circle_filled(c, 3.0, theme::BLUE);
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(40));
        }
        StepState::Pending => {
            p.circle_stroke(c, 10.0, Stroke::new(1.6, Color32::from_rgb(0x3A, 0x45, 0x58)));
        }
        StepState::Failed => {
            p.circle_filled(c, 11.0, theme::RED);
            let s = Stroke::new(2.4, theme::BG);
            p.line_segment([pos2(c.x - 4.0, c.y - 4.0), pos2(c.x + 4.0, c.y + 4.0)], s);
            p.line_segment([pos2(c.x - 4.0, c.y + 4.0), pos2(c.x + 4.0, c.y - 4.0)], s);
        }
    }
}

fn big_check(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
    let c = rect.center();
    ui.painter().circle_filled(c, 30.0, Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 40));
    ui.painter().circle_stroke(c, 30.0, Stroke::new(2.5, color));
    let s = Stroke::new(4.0, color);
    ui.painter().line_segment([pos2(c.x - 13.0, c.y + 1.0), pos2(c.x - 4.0, c.y + 10.0)], s);
    ui.painter().line_segment([pos2(c.x - 4.0, c.y + 10.0), pos2(c.x + 14.0, c.y - 9.0)], s);
}

fn narrow(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let w = ui.available_width().min(700.0);
    ui.allocate_ui(Vec2::new(w, ui.available_height()), |ui| {
        ui.set_width(w);
        add(ui);
    });
}

fn log_box(ui: &mut Ui, text: &str, height: f32, stick: bool) {
    egui::Frame::new().fill(Color32::from_rgb(0x0B, 0x0E, 0x13)).stroke(Stroke::new(1.0, theme::LINE)).corner_radius(egui::CornerRadius::same(8)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ScrollArea::both().max_height(height).min_scrolled_height(height).stick_to_bottom(stick).auto_shrink([false, false]).id_salt("log").show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(text).monospace().size(12.5).color(Color32::from_rgb(0xBF, 0xC8, 0xD6))).wrap_mode(egui::TextWrapMode::Extend));
        });
    });
}

impl App {
    /// A snapshot of the worker for drawing (the Progress is cloned: it is small except for the log, which is only copied while the details are open).
    fn run_snapshot(&self) -> Option<(Progress, std::time::Instant)> {
        self.engine.as_ref().map(|e| (e.progress.clone(), e.started)).or_else(|| self.demo_run.clone())
    }

    pub fn show_run(&mut self, ui: &mut Ui) {
        let Some((pr, started)) = self.run_snapshot() else {
            ui.label("...");
            return;
        };
        self.show_run_with(ui, &pr, started);
    }

    /// The install page for a given progress (also used by the snapshot tests with a demo progress).
    pub fn show_run_with(&mut self, ui: &mut Ui, pr: &Progress, started: std::time::Instant) {
        let names = pr.step_names();
        let pct = pr.progress.floor().min(100.0) as u32;
        let note = match self.run_kind {
            RunKind::Update { .. } => "This takes about 2 to 10 minutes. You can keep using your PC.",
            _ => "This takes about 15 minutes. You can keep using your PC.",
        };
        let mut cancel_ask = false;
        let mut cancel_yes = false;
        let mut cancel_no = false;
        let mut toggle = false;
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            narrow(ui, |ui| {
                theme::card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(theme::bold(self.run_title(), 22.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if !self.confirm_cancel {
                                cancel_ask = theme::button(ui, "Cancel");
                            }
                        });
                    });
                    if self.confirm_cancel {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new("Stop the setup? You can start it again later and it continues.").color(theme::ORANGE));
                            cancel_yes = theme::button(ui, "Yes, stop");
                            cancel_no = theme::button(ui, "Keep going");
                        });
                    }
                    ui.add_space(6.0);
                    for (i, n) in names.iter().enumerate() {
                        let st = if pr.finished || i < pr.step {
                            StepState::Done
                        } else if i == pr.step {
                            StepState::Active
                        } else {
                            StepState::Pending
                        };
                        ui.horizontal(|ui| {
                            step_icon(ui, st);
                            let t = match st {
                                StepState::Active => theme::bold(*n, 16.0),
                                StepState::Pending => RichText::new(*n).size(16.0).color(theme::MUTED),
                                _ => RichText::new(*n).size(16.0),
                            };
                            ui.label(t);
                        });
                    }
                    ui.add_space(8.0);
                    theme::progress(ui, pr.progress as f32 / 100.0, 12.0, theme::ORANGE);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("Elapsed: {}", format_elapsed(started.elapsed()))).size(14.5));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(theme::bold(format!("{pct}%"), 15.0));
                        });
                    });
                    ui.label(theme::muted(truncate(&pr.status, 110)));
                    ui.label(RichText::new(note).size(14.5));
                    toggle = theme::link(ui, if self.show_details { "Hide details" } else { "Show details" });
                });
                if self.show_details {
                    let text = tail(&pr.log, 400);
                    log_box(ui, &text, 150.0, true);
                }
            });
        });
        if toggle {
            self.show_details = !self.show_details;
        }
        if cancel_ask {
            self.confirm_cancel = true;
        }
        if cancel_no {
            self.confirm_cancel = false;
        }
        if cancel_yes {
            self.cancel_run();
        }
    }

    pub fn show_failed(&mut self, ui: &mut Ui) {
        let Some(f) = self.failure.as_ref() else { return };
        let (msg, details, log, is_update) = (f.message.clone(), f.details.clone(), f.log.clone(), f.update);
        let (mut retry, mut close, mut copy, mut play_backup) = (false, false, false, false);
        let backup = if is_update { self.backup_for_failure() } else { None };
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            narrow(ui, |ui| {
                ui.horizontal(|ui| {
                    step_icon(ui, StepState::Failed);
                    ui.label(theme::bold("Something went wrong", 22.0).color(theme::RED));
                });
                theme::card_colored(ui, theme::RED, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(msg).size(15.0));
                });
                if !log.is_empty() {
                    ui.label(theme::muted("Details"));
                    let t = tail_text(&log, 300);
                    log_box(ui, &t, 180.0, true);
                }
                ui.horizontal(|ui| {
                    if !details.is_empty() {
                        copy = theme::button(ui, "Copy details");
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        retry = theme::primary_button(ui, "Try again", Vec2::new(140.0, 40.0), true);
                        if let Some((_, v)) = &backup {
                            play_backup = theme::button_enabled(ui, &format!("Play the backup (v{v})"), !self.game_running());
                        }
                        close = theme::button(ui, if is_update { "Play old version" } else { "Back" });
                    });
                });
            });
        });
        if copy {
            ui.ctx().copy_text(details);
            self.toast("Details copied. Paste them when you ask for help.", false);
        }
        if retry {
            self.retry_after_failure();
        }
        if play_backup {
            let ctx = ui.ctx().clone();
            self.play_the_backup(&ctx);
        }
        if close {
            self.failure = None;
            self.view = View::Home;
            self.refresh_info();
            if is_update {
                let ctx = ui.ctx().clone();
                self.play(&ctx);
            }
        }
    }

    fn retry_after_failure(&mut self) {
        let update = self.failure.as_ref().map(|f| f.update).unwrap_or(false);
        self.failure = None;
        if update {
            if let Some((to, finish)) = self.last_update.clone() {
                self.start_update(&to, finish);
                return;
            }
        }
        self.view = if self.run_kind == RunKind::Repair { View::Repair } else { View::Home };
    }

    pub fn show_done(&mut self, ui: &mut Ui) {
        let title = match &self.run_kind {
            RunKind::Update { to, .. } => format!("Updated to v{to}"),
            RunKind::Repair => "Repair finished".to_string(),
            RunKind::Install => "Ready to play".to_string(),
        };
        let note = self.done_note.clone();
        let mut play = false;
        let mut back = false;
        narrow(ui, |ui| {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                big_check(ui, theme::GREEN);
                ui.add_space(6.0);
                ui.label(theme::bold(title, 28.0));
                ui.label(RichText::new("Everything is set up. Click Play to start the game.").size(15.0));
                ui.add_space(10.0);
                play = theme::primary_button(ui, "Play", Vec2::new(300.0, 76.0), !self.game_running());
                if !note.is_empty() {
                    ui.add_space(6.0);
                    ui.label(theme::muted(note));
                }
                ui.add_space(6.0);
                back = theme::button(ui, "Settings and mods");
            });
        });
        if play {
            let ctx = ui.ctx().clone();
            self.view = View::Home;
            self.play(&ctx);
        }
        if back {
            self.view = View::Home;
            self.tab = super::state::Tab::Settings;
        }
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(n).collect::<String>())
    }
}

fn tail(lines: &[String], n: usize) -> String {
    let from = lines.len().saturating_sub(n);
    lines[from..].join("\n")
}

fn tail_text(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let from = lines.len().saturating_sub(n);
    lines[from..].join("\n")
}
