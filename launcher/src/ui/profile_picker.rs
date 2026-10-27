//! The profile picker: a button ("Profile: Default") that opens a small list of the install's profiles with New / Copy / Rename / Delete.
//! Shown next to the Play button (big) and at the top of the Settings and Mods pages (small).
use super::state::App;
use super::state_lib::EditKind;
use super::theme;
use egui::{Align2, Area, Color32, CornerRadius, FontId, Frame, Id, Margin, Order, Pos2, RichText, Sense, Stroke, Ui, Vec2, WidgetInfo};

const POPUP_W: f32 = 320.0;

/// A small painted chevron (no font glyph needed).
fn chevron(ui: &Ui, center: Pos2, color: Color32) {
    let pts = vec![center + Vec2::new(-5.0, -2.5), center + Vec2::new(5.0, -2.5), center + Vec2::new(0.0, 3.5)];
    ui.painter().add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
}

impl App {
    /// The picker button. `big` = the tall version for the Play tab. The list opens below it (`profile_popup`).
    pub fn profile_picker(&mut self, ui: &mut Ui, big: bool, width: f32) {
        let name = self.profile_name();
        let enabled = !self.read_only();
        let h = if big { 72.0 } else { 38.0 };
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, h), Sense::click());
        self.profile_ui.anchor = Some(rect);
        let label = format!("Profile: {name}");
        resp.widget_info(|| WidgetInfo::labeled(egui::Role::Button, enabled, &label));
        let open = self.profile_ui.open;
        let fill = if !enabled {
            theme::CARD
        } else if open || resp.hovered() {
            theme::CARD_HI
        } else {
            theme::CARD
        };
        let p = ui.painter();
        p.rect(rect, CornerRadius::same(12), fill, Stroke::new(1.0, if open { theme::ORANGE } else { theme::LINE }), egui::StrokeKind::Inside);
        let text_col = if enabled { theme::TEXT } else { theme::MUTED };
        if big {
            p.text(rect.left_top() + Vec2::new(16.0, 14.0), Align2::LEFT_TOP, "PROFILE", FontId::proportional(11.5), theme::MUTED);
            p.text(rect.left_top() + Vec2::new(16.0, 33.0), Align2::LEFT_TOP, ellipsis(&name, 20), FontId::new(18.0, theme::bold_family()), text_col);
        } else {
            p.text(rect.left_center() + Vec2::new(14.0, 0.0), Align2::LEFT_CENTER, format!("Profile: {}", ellipsis(&name, 22)), FontId::new(14.5, theme::bold_family()), text_col);
        }
        chevron(ui, Pos2::new(rect.right() - 18.0, rect.center().y), if enabled { theme::ORANGE } else { theme::MUTED });
        if enabled && resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() && enabled {
            self.profile_ui.open = !self.profile_ui.open;
            self.profile_ui.edit = None;
            self.profile_ui.error = None;
        }
        if self.profile_ui.open && !enabled {
            self.profile_ui.open = false;
        }
    }

    /// Draws the open list under the picker button (call once per frame after the page; a no-op when the picker is closed).
    pub fn profile_popup(&mut self, ctx: &egui::Context) {
        if !self.profile_ui.open {
            return;
        }
        let Some(anchor) = self.profile_ui.anchor else {
            self.profile_ui.open = false;
            return;
        };
        let pos = Pos2::new((anchor.left()).min(ctx.content_rect().right() - POPUP_W - 8.0).max(8.0), anchor.bottom() + 6.0);
        let mut choose: Option<String> = None;
        let mut begin: Option<EditKind> = None;
        let mut apply = false;
        let mut cancel_edit = false;
        #[allow(deprecated)]
        let out = Area::new(Id::new("profile_popup")).order(Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
            Frame::new().fill(theme::CARD_HI).stroke(Stroke::new(1.0, theme::ORANGE)).corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(12)).show(ui, |ui| {
                ui.set_width(POPUP_W - 24.0);
                ui.label(RichText::new("PROFILES").size(11.5).color(theme::MUTED).strong());
                ui.add_space(2.0);
                let list = self.profile_list.clone();
                for p in &list {
                    let sel = p.id == self.profile_id;
                    let sub = if p.meta.last_used.is_empty() { format!("made {}", p.meta.created) } else { format!("played {}", p.meta.last_used) };
                    if profile_row(ui, sel, &p.meta.name, &sub) {
                        choose = Some(p.id.clone());
                    }
                }
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(4.0);
                if self.profile_ui.edit.is_none() {
                    ui.horizontal_wrapped(|ui| {
                        for (k, t) in [(EditKind::New, "New"), (EditKind::Copy, "Copy"), (EditKind::Rename, "Rename")] {
                            if theme::button(ui, t) {
                                begin = Some(k);
                            }
                        }
                        if theme::button_enabled(ui, "Delete", list.len() > 1) {
                            begin = Some(EditKind::Delete);
                        }
                    });
                    ui.label(theme::muted("Each profile has its own settings and mods list. Your saves are shared by all profiles."));
                } else {
                    self.profile_form(ui, &mut apply, &mut cancel_edit);
                }
            });
        });
        if let Some(id) = choose {
            self.select_profile(&id);
            self.profile_ui.open = false;
        }
        if let Some(k) = begin {
            self.profile_begin(k);
        }
        if apply {
            self.profile_apply();
        }
        if cancel_edit {
            self.profile_ui.edit = None;
            self.profile_ui.error = None;
        }
        // a click outside the list and the button, or Escape, closes it
        let clicked_outside = ctx.input(|i| i.pointer.any_pressed()) && ctx.pointer_interact_pos().map(|p| !out.response.rect.contains(p) && !anchor.contains(p)).unwrap_or(false);
        if clicked_outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.profile_ui.open = false;
            self.profile_ui.edit = None;
            self.profile_ui.error = None;
        }
    }

    fn profile_form(&mut self, ui: &mut Ui, apply: &mut bool, cancel: &mut bool) {
        let Some(edit) = self.profile_ui.edit.as_mut() else { return };
        let kind = edit.kind;
        match kind {
            EditKind::Delete => {
                ui.label(theme::bold(format!("Delete \"{}\"?", self.profile_name()), 15.0));
                ui.label(theme::muted("Its settings and mods list are removed. The mods themselves and your saves stay."));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    *apply = theme::button(ui, "Yes, delete it");
                    *cancel = theme::button(ui, "Cancel");
                });
            }
            _ => {
                let title = match kind {
                    EditKind::New => "New profile (the game's own settings, no mods)",
                    EditKind::Copy => "Copy of this profile",
                    _ => "Rename this profile",
                };
                ui.label(theme::bold(title, 14.0));
                let r = ui.add(egui::TextEdit::singleline(&mut edit.text).hint_text("Profile name").desired_width(ui.available_width()).margin(Margin::symmetric(10, 7)));
                r.widget_info(|| WidgetInfo::text_edit(true, "", "", "Profile name"));
                if !r.has_focus() && !r.lost_focus() {
                    r.request_focus();
                }
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let label = match kind {
                        EditKind::New => "Create",
                        EditKind::Copy => "Copy",
                        _ => "Rename",
                    };
                    *apply = theme::button(ui, label) || enter;
                    *cancel = theme::button(ui, "Cancel");
                });
            }
        }
        if let Some(e) = &self.profile_ui.error {
            ui.label(RichText::new(e.clone()).color(theme::RED).size(13.5));
        }
    }

    /// The line at the top of Settings and Mods: which profile is edited (and a note while the game runs).
    pub fn profile_header(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            self.profile_picker(ui, false, 250.0);
            if self.read_only() {
                ui.label(RichText::new("The game is running: settings and mods can be changed after you close it.").color(theme::ORANGE).size(13.5));
            } else {
                ui.label(theme::muted("These changes belong to this profile."));
            }
        });
    }
}

fn ellipsis(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(max.saturating_sub(3)).collect::<String>())
    }
}

/// One profile in the list; true when clicked.
fn profile_row(ui: &mut Ui, selected: bool, name: &str, sub: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 42.0), Sense::click());
    resp.widget_info(|| WidgetInfo::selected(egui::Role::RadioButton, true, selected, format!("Profile {name}")));
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, CornerRadius::same(8), theme::CARD);
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(8), theme::PANEL);
    }
    p.circle_stroke(Pos2::new(rect.left() + 16.0, rect.center().y), 6.0, Stroke::new(1.5, if selected { theme::ORANGE } else { theme::MUTED }));
    if selected {
        p.circle_filled(Pos2::new(rect.left() + 16.0, rect.center().y), 3.5, theme::ORANGE);
    }
    p.text(rect.left_top() + Vec2::new(34.0, 5.0), Align2::LEFT_TOP, ellipsis(name, 26), FontId::new(15.0, theme::bold_family()), theme::TEXT);
    p.text(rect.left_top() + Vec2::new(34.0, 24.0), Align2::LEFT_TOP, sub, FontId::proportional(12.0), theme::MUTED);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.clicked()
}
