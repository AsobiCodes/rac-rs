//! The picture of a game: the cover PCSX2 downloaded for the player (Documents\PCSX2\covers\<serial>.jpg|png) when it exists, otherwise a
//! title card drawn in code. No game art is shipped with the launcher.
use crate::platform;
use egui::{pos2, Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, TextureHandle, Ui, Vec2};
use std::collections::HashMap;
use std::path::PathBuf;

use super::theme;

#[derive(Default)]
pub struct Covers {
    tex: HashMap<String, Option<TextureHandle>>,
    /// Folder that holds the covers (tests point it elsewhere).
    pub dir: Option<PathBuf>,
}

fn cover_dir(dir: &Option<PathBuf>) -> Option<PathBuf> {
    dir.clone().or_else(|| platform::known_folder(platform::Known::Documents).map(|d| d.join("PCSX2").join("covers")))
}

impl Covers {
    fn texture(&mut self, ctx: &egui::Context, serials: &[&str]) -> Option<TextureHandle> {
        let key = serials.join(",");
        if let Some(t) = self.tex.get(&key) {
            return t.clone();
        }
        let mut found = None;
        if let Some(dir) = cover_dir(&self.dir) {
            'outer: for s in serials {
                for ext in ["jpg", "jpeg", "png"] {
                    let p = dir.join(format!("{s}.{ext}"));
                    if let Ok(bytes) = std::fs::read(&p) {
                        if let Ok(img) = image::load_from_memory(&bytes) {
                            let img = img.thumbnail(640, 640).to_rgba8();
                            let size = [img.width() as usize, img.height() as usize];
                            let ci = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                            found = Some(ctx.load_texture(format!("cover-{s}"), ci, egui::TextureOptions::LINEAR));
                            break 'outer;
                        }
                    }
                }
            }
        }
        self.tex.insert(key, found.clone());
        found
    }

    /// Draws the cover (or the title card) in a box of `size`.
    pub fn show(&mut self, ui: &mut Ui, serials: &[&str], title: &str, size: Vec2) {
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        match self.texture(ui.ctx(), serials) {
            Some(t) => {
                let ts = t.size_vec2();
                let scale = (size.x / ts.x).min(size.y / ts.y);
                let r = Rect::from_center_size(rect.center(), ts * scale);
                let shadow = r.translate(Vec2::new(0.0, 6.0)).expand(2.0);
                ui.painter().rect_filled(shadow, CornerRadius::same(8), Color32::from_black_alpha(90));
                egui::Image::new(&t).corner_radius(CornerRadius::same(8)).paint_at(ui, r);
            }
            None => title_card(ui, rect, title),
        }
    }
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// A card made of simple shapes and text: sky gradient, a ringed planet, stars.
pub fn title_card(ui: &mut Ui, rect: Rect, title: &str) {
    let p = ui.painter().with_clip_rect(rect);
    let (top, mid, bottom) = (Color32::from_rgb(0x0B, 0x1C, 0x3F), Color32::from_rgb(0x1B, 0x3F, 0x8C), Color32::from_rgb(0x8A, 0x3A, 0x12));
    let n = 40;
    for i in 0..n {
        let t0 = i as f32 / n as f32;
        let t1 = (i + 1) as f32 / n as f32;
        let col = if t0 < 0.6 { lerp(top, mid, t0 / 0.6) } else { lerp(mid, bottom, (t0 - 0.6) / 0.4) };
        let strip = Rect::from_min_max(pos2(rect.left(), rect.top() + rect.height() * t0), pos2(rect.right(), (rect.top() + rect.height() * t1 + 1.0).min(rect.bottom())));
        let r = if i == 0 {
            CornerRadius { nw: 12, ne: 12, sw: 0, se: 0 }
        } else if i == n - 1 {
            CornerRadius { nw: 0, ne: 0, sw: 12, se: 12 }
        } else {
            CornerRadius::ZERO
        };
        p.rect_filled(strip, r, col);
    }
    // stars (fixed positions)
    for k in 0..26u32 {
        let x = ((k * 7919 + 13) % 997) as f32 / 997.0;
        let y = ((k * 104729 + 7) % 991) as f32 / 991.0 * 0.62;
        let a = 70 + (k * 37 % 140) as u8;
        p.circle_filled(pos2(rect.left() + rect.width() * x, rect.top() + rect.height() * y), 0.9 + (k % 3) as f32 * 0.5, Color32::from_white_alpha(a));
    }
    // planet with a ring
    let c = pos2(rect.center().x + rect.width() * 0.12, rect.top() + rect.height() * 0.40);
    let r = rect.width() * 0.26;
    p.circle_filled(c, r + 6.0, Color32::from_rgba_unmultiplied(0xFF, 0x8A, 0x1F, 40));
    p.circle_filled(c, r, theme::ORANGE);
    p.circle_filled(c + Vec2::new(-r * 0.28, -r * 0.28), r * 0.72, theme::ORANGE_HI);
    p.circle_filled(c + Vec2::new(r * 0.30, r * 0.30), r * 0.55, theme::ORANGE_LO);
    let ring: Vec<_> = (0..=48).map(|i| {
        let a = i as f32 / 48.0 * std::f32::consts::TAU;
        pos2(c.x + a.cos() * r * 1.55, c.y + a.sin() * r * 0.32 - r * 0.15 + a.cos() * r * 0.2)
    }).collect();
    p.add(egui::Shape::line(ring, Stroke::new(3.0, Color32::from_rgba_unmultiplied(0x9F, 0xD4, 0xFF, 200))));
    // title
    let lines: Vec<&str> = title.split(" & ").collect();
    let big = FontId::new((rect.width() * 0.125).clamp(18.0, 40.0), theme::bold_family());
    let y0 = rect.top() + rect.height() * 0.72;
    if lines.len() == 2 {
        p.text(pos2(rect.center().x, y0), Align2::CENTER_CENTER, lines[0].to_uppercase(), big.clone(), Color32::WHITE);
        p.text(pos2(rect.center().x, y0 + big.size * 0.95), Align2::CENTER_CENTER, format!("& {}", lines[1].to_uppercase()), big.clone(), theme::BLUE);
    } else {
        p.text(pos2(rect.center().x, y0), Align2::CENTER_CENTER, title.to_uppercase(), big, Color32::WHITE);
    }
    ui.painter().rect_stroke(rect, CornerRadius::same(12), Stroke::new(1.0, theme::LINE), StrokeKind::Inside);
}
