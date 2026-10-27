//! Look of the launcher: dark theme with Ratchet orange and Clank blue accents, the Segoe UI font when Windows has it, and the few
//! widgets every page shares (big primary button, quiet button, cards, progress bar, chips, segmented choice).
use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};

pub const BG: Color32 = Color32::from_rgb(0x0F, 0x12, 0x18);
pub const SIDEBAR: Color32 = Color32::from_rgb(0x13, 0x17, 0x1F);
pub const PANEL: Color32 = Color32::from_rgb(0x17, 0x1C, 0x26);
pub const CARD: Color32 = Color32::from_rgb(0x1D, 0x24, 0x30);
pub const CARD_HI: Color32 = Color32::from_rgb(0x26, 0x2F, 0x3E);
pub const LINE: Color32 = Color32::from_rgb(0x2B, 0x34, 0x45);
pub const TEXT: Color32 = Color32::from_rgb(0xE8, 0xEC, 0xF2);
pub const MUTED: Color32 = Color32::from_rgb(0x93, 0x9E, 0xB1);
pub const ORANGE: Color32 = Color32::from_rgb(0xFF, 0x8A, 0x1F);
pub const ORANGE_HI: Color32 = Color32::from_rgb(0xFF, 0xA4, 0x4D);
pub const ORANGE_LO: Color32 = Color32::from_rgb(0xD9, 0x6C, 0x08);
pub const BLUE: Color32 = Color32::from_rgb(0x3F, 0xA9, 0xF5);
pub const GREEN: Color32 = Color32::from_rgb(0x4C, 0xC3, 0x8A);
pub const RED: Color32 = Color32::from_rgb(0xF0, 0x64, 0x5A);
pub const ON_ORANGE: Color32 = Color32::from_rgb(0x1A, 0x10, 0x05);

pub fn bold_family() -> FontFamily {
    FontFamily::Name("bold".into())
}

pub fn bold(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).font(FontId::new(size, bold_family())).color(TEXT)
}

pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text).color(MUTED).size(13.5)
}

fn load_font(name: &str) -> Option<egui::FontData> {
    let win = std::env::var_os("SystemRoot").map(std::path::PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
    std::fs::read(win.join("Fonts").join(name)).ok().map(egui::FontData::from_owned)
}

/// Segoe UI (regular + semibold) from the Windows fonts folder when it exists; egui's own fonts stay as the fallback (also on Linux).
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let regular = load_font("segoeui.ttf");
    let semi = load_font("seguisb.ttf").or_else(|| load_font("segoeuib.ttf"));
    if let Some(r) = regular {
        fonts.font_data.insert("segoe".into(), std::sync::Arc::new(r));
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "segoe".into());
    }
    let mut bold_list: Vec<String> = Vec::new();
    if let Some(b) = semi {
        fonts.font_data.insert("segoe-bold".into(), std::sync::Arc::new(b));
        bold_list.push("segoe-bold".into());
    }
    bold_list.extend(fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
    fonts.families.insert(bold_family(), bold_list);
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = Color32::from_rgb(0x0B, 0x0E, 0x13);
    v.faint_bg_color = CARD;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = BLUE;
    v.selection.bg_fill = Color32::from_rgba_unmultiplied(0x3F, 0xA9, 0xF5, 90);
    v.selection.stroke = Stroke::new(1.0, BLUE);
    let r = CornerRadius::same(8);
    v.window_corner_radius = CornerRadius::same(12);
    v.widgets.noninteractive.bg_fill = CARD;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.bg_fill = CARD_HI;
    v.widgets.inactive.weak_bg_fill = CARD_HI;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.bg_fill = Color32::from_rgb(0x30, 0x3B, 0x4E);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x30, 0x3B, 0x4E);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, BLUE);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.bg_fill = Color32::from_rgb(0x38, 0x45, 0x5A);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(0x38, 0x45, 0x5A);
    v.widgets.active.bg_stroke = Stroke::new(1.0, BLUE);
    v.widgets.active.corner_radius = r;
    v.widgets.open = v.widgets.hovered;
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = Vec2::new(10.0, 10.0);
        s.spacing.button_padding = Vec2::new(14.0, 8.0);
        s.spacing.interact_size.y = 32.0;
        s.spacing.scroll.bar_width = 8.0;
        use egui::TextStyle::*;
        s.text_styles.insert(Body, FontId::new(15.0, FontFamily::Proportional));
        s.text_styles.insert(Button, FontId::new(15.0, FontFamily::Proportional));
        s.text_styles.insert(Small, FontId::new(12.5, FontFamily::Proportional));
        s.text_styles.insert(Heading, FontId::new(26.0, bold_family()));
        s.text_styles.insert(Monospace, FontId::new(12.5, FontFamily::Monospace));
    });
}

/// The big orange button. Returns the click.
pub fn primary_button(ui: &mut Ui, text: &str, size: Vec2, enabled: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let fill = if !enabled {
        Color32::from_rgb(0x3A, 0x30, 0x27)
    } else if resp.is_pointer_button_down_on() {
        ORANGE_LO
    } else if resp.hovered() {
        ORANGE_HI
    } else {
        ORANGE
    };
    let radius = CornerRadius::same((size.y * 0.22).clamp(8.0, 16.0) as u8);
    let p = ui.painter();
    if enabled && resp.hovered() {
        p.rect_filled(rect.expand(3.0), radius, Color32::from_rgba_unmultiplied(0xFF, 0x8A, 0x1F, 50));
    }
    p.rect_filled(rect, radius, fill);
    let size_pt = (size.y * 0.36).clamp(15.0, 30.0);
    let col = if enabled { ON_ORANGE } else { Color32::from_rgb(0x8A, 0x7A, 0x6A) };
    p.text(rect.center(), egui::Align2::CENTER_CENTER, text, FontId::new(size_pt, bold_family()), col);
    if enabled && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    enabled && resp.clicked()
}

/// A quiet button (dark, blue edge on hover).
pub fn button(ui: &mut Ui, text: &str) -> bool {
    ui.add(egui::Button::new(RichText::new(text).size(14.5))).clicked()
}

pub fn button_enabled(ui: &mut Ui, text: &str, enabled: bool) -> bool {
    ui.add_enabled(enabled, egui::Button::new(RichText::new(text).size(14.5))).clicked()
}

/// A link-like text button.
pub fn link(ui: &mut Ui, text: &str) -> bool {
    let r = ui.add(egui::Label::new(RichText::new(text).color(BLUE).size(14.0)).sense(Sense::click()));
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    r.clicked()
}

pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<R> {
    egui::Frame::new().fill(CARD).stroke(Stroke::new(1.0, LINE)).corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(16)).show(ui, add)
}

pub fn card_colored<R>(ui: &mut Ui, edge: Color32, add: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<R> {
    egui::Frame::new().fill(CARD).stroke(Stroke::new(1.5, edge)).corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(16)).show(ui, add)
}

/// A small rounded label ("Coming soon", "Texture pack", "Update").
pub fn chip(ui: &mut Ui, text: &str, color: Color32) {
    let galley = ui.painter().layout_no_wrap(text.to_string(), FontId::new(12.0, bold_family()), color);
    let size = galley.size() + Vec2::new(16.0, 6.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(10), Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 38));
    ui.painter().rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 110)), StrokeKind::Inside);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
}

/// A thin progress bar (0..=1) over the full width.
pub fn progress(ui: &mut Ui, frac: f32, height: f32, color: Color32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, height), Sense::hover());
    let r = CornerRadius::same((height / 2.0) as u8);
    ui.painter().rect_filled(rect, r, Color32::from_rgb(0x25, 0x2D, 0x3B));
    let fw = (rect.width() * frac.clamp(0.0, 1.0)).max(if frac > 0.0 { height } else { 0.0 });
    if fw > 0.0 {
        let fill = egui::Rect::from_min_size(rect.min, Vec2::new(fw.min(rect.width()), height));
        ui.painter().rect_filled(fill, r, color);
    }
}

/// An endless bar for work whose size is not known yet: a short bar sweeps to and fro (keeps the window repainting while it is shown).
pub fn progress_busy(ui: &mut Ui, height: f32, color: Color32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, height), Sense::hover());
    let r = CornerRadius::same((height / 2.0) as u8);
    ui.painter().rect_filled(rect, r, Color32::from_rgb(0x25, 0x2D, 0x3B));
    let seg = (rect.width() * 0.3).max(height);
    let t = ui.input(|i| i.time) as f32;
    let k = 0.5 - 0.5 * (t * 2.4).cos();
    let fill = egui::Rect::from_min_size(egui::pos2(rect.min.x + (rect.width() - seg).max(0.0) * k, rect.min.y), Vec2::new(seg.min(rect.width()), height));
    ui.painter().rect_filled(fill, r, color);
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
}

fn seg_font(on: bool) -> FontId {
    FontId::new(14.0, if on { bold_family() } else { FontFamily::Proportional })
}

/// Width a segmented control needs.
pub fn segmented_width(ui: &Ui, labels: &[&str]) -> f32 {
    labels.iter().map(|l| (ui.painter().layout_no_wrap(l.to_string(), seg_font(true), TEXT).size().x + 24.0).max(54.0)).sum::<f32>() + 3.0 * labels.len().saturating_sub(1) as f32
}

/// A row of exclusive choices drawn as one segmented control (one widget, so it can be right-aligned). `selected` = the current choice.
/// `name` is what a screen reader (and the UI tests) call the group; each segment is announced as "<name>: <label>".
pub fn segmented(ui: &mut Ui, id: &str, name: &str, labels: &[&str], selected: Option<usize>) -> Option<usize> {
    segmented_gated(ui, id, name, labels, selected, &[])
}

/// Like `segmented`, but the segments flagged in `locked` (same order as `labels`; missing = not locked) are greyed and cannot be clicked.
pub fn segmented_gated(ui: &mut Ui, id: &str, name: &str, labels: &[&str], selected: Option<usize>, locked: &[bool]) -> Option<usize> {
    let total = segmented_width(ui, labels);
    let (outer, _) = ui.allocate_exact_size(Vec2::new(total, 30.0), Sense::hover());
    let mut clicked = None;
    let mut x = outer.left();
    for (i, l) in labels.iter().enumerate() {
        let on = selected == Some(i);
        let off = locked.get(i).copied().unwrap_or(false);
        let w = (ui.painter().layout_no_wrap(l.to_string(), seg_font(true), TEXT).size().x + 24.0).max(54.0);
        let rect = egui::Rect::from_min_size(egui::pos2(x, outer.top()), Vec2::new(w, 30.0));
        x += w + 3.0;
        let resp = ui.interact(rect, ui.scope_id().with((id, i)), if off { Sense::hover() } else { Sense::click() });
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, !off, on, format!("{name}: {l}")));
        let (fill, col) = if off {
            (Color32::from_rgb(0x1B, 0x21, 0x2B), Color32::from_rgb(0x58, 0x61, 0x70))
        } else if on {
            (ORANGE, ON_ORANGE)
        } else if resp.hovered() {
            (Color32::from_rgb(0x30, 0x3B, 0x4E), TEXT)
        } else {
            (Color32::from_rgb(0x22, 0x2A, 0x37), MUTED)
        };
        ui.painter().rect_filled(rect, CornerRadius::same(7), fill);
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, l, seg_font(on && !off), col);
        if off && on {
            // the file says this value but this game version ignores it: a thin outline instead of the orange fill
            ui.painter().rect_stroke(rect, CornerRadius::same(7), egui::Stroke::new(1.0, ORANGE.gamma_multiply(0.6)), egui::StrokeKind::Inside);
        }
        if resp.hovered() && !off {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// An on/off switch. Returns true when clicked (the caller flips its value). `name` is its accessible name.
pub fn switch(ui: &mut Ui, on: bool, enabled: bool, name: &str) -> bool {
    let size = Vec2::new(44.0, 24.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, on, name));
    let t = ui.ctx().animate_bool_responsive(resp.id, on);
    let track = if !enabled {
        Color32::from_rgb(0x2A, 0x30, 0x3B)
    } else if on {
        ORANGE
    } else {
        Color32::from_rgb(0x3A, 0x45, 0x58)
    };
    ui.painter().rect_filled(rect, CornerRadius::same(12), track);
    let cx = rect.left() + 12.0 + t * (size.x - 24.0);
    ui.painter().circle_filled(egui::pos2(cx, rect.center().y), 9.0, if enabled { Color32::WHITE } else { Color32::from_rgb(0x70, 0x78, 0x86) });
    enabled && resp.clicked()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Up,
    Down,
}

/// A small square button with a drawn triangle (no font needed).
pub fn icon_button(ui: &mut Ui, icon: Icon, enabled: bool, tip: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(30.0), if enabled { Sense::click() } else { Sense::hover() });
    let fill = if enabled && resp.hovered() { Color32::from_rgb(0x30, 0x3B, 0x4E) } else { Color32::from_rgb(0x22, 0x2A, 0x37) };
    ui.painter().rect_filled(rect, CornerRadius::same(7), fill);
    let col = if enabled { TEXT } else { Color32::from_rgb(0x4A, 0x53, 0x62) };
    let c = rect.center();
    let s = 5.5;
    let pts = match icon {
        Icon::Up => vec![egui::pos2(c.x - s, c.y + s * 0.6), egui::pos2(c.x + s, c.y + s * 0.6), egui::pos2(c.x, c.y - s * 0.7)],
        Icon::Down => vec![egui::pos2(c.x - s, c.y - s * 0.6), egui::pos2(c.x + s, c.y - s * 0.6), egui::pos2(c.x, c.y + s * 0.7)],
    };
    ui.painter().add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
    resp.clone().on_hover_text(tip);
    enabled && resp.clicked()
}
