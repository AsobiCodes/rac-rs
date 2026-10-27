//! The two master ticks at the top of the Mods page ("Load texture packs", "Load mods"), like the on/off switches other mod launchers have.
//! A tick off keeps every per-mod switch below it; the game just does not load that kind (see `mods::active` for how it reaches the game).
use super::state::App;
use super::theme;
use crate::gating;
use crate::mods::Master;
use egui::{RichText, Ui};

/// What one tick shows.
struct MasterRow {
    master: Master,
    on: bool,
    /// False = greyed: the game version cannot use the kind yet, nothing of that kind is installed, or the game is running.
    enabled: bool,
    text: String,
    /// The text is a reason (shown in orange) and not a status.
    reason: bool,
}

/// "2 of 3 packs are switched on below." / "No texture packs yet."
fn status_text(m: Master, on: bool, installed: usize, switched_on: usize) -> String {
    let (noun, none) = match m {
        Master::Textures => ("packs", "No texture packs yet. Add one below."),
        Master::Mods => ("mods", "No script mods yet. Put a mod's folder into the mods folder and it shows up below."),
    };
    if installed == 0 {
        return none.to_string();
    }
    if !on {
        return format!("Off: the game loads no {noun}. Your switches below are kept for when you switch this on.");
    }
    format!("{switched_on} of {installed} {noun} {} switched on below.", if switched_on == 1 { "is" } else { "are" })
}

impl App {
    fn master_rows(&self) -> Vec<MasterRow> {
        let store = &self.mods_ui.store;
        let running = self.read_only();
        Master::ALL
            .into_iter()
            .map(|m| {
                let (installed, switched_on) = store.master_counts(m);
                let on = store.master_on(m);
                // the game reads the packs, the script mods, active.txt and settings.json from 0.4.0 on
                let feature = if m == Master::Textures { gating::TEXTURE_PACKS } else { gating::SCRIPT_MODS };
                let locked = gating::needs_text(&self.info.local_version, feature);
                let empty = installed == 0;
                match locked {
                    Some(t) => MasterRow { master: m, on, enabled: false, text: t, reason: true },
                    None => MasterRow { master: m, on, enabled: !running && !(empty && on), text: status_text(m, on, installed, switched_on), reason: false },
                }
            })
            .collect()
    }

    /// Shown when there is something to switch (a mod is installed, or a tick is off and must be switchable back on).
    pub fn mods_master_card(&mut self, ui: &mut Ui) {
        let store = &self.mods_ui.store;
        if store.entries.is_empty() && !store.masters.any_off() {
            return;
        }
        let rows = self.master_rows();
        let mut flip: Option<(Master, bool)> = None;
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            for (i, r) in rows.iter().enumerate() {
                if i > 0 {
                    ui.add_space(2.0);
                    ui.separator();
                    ui.add_space(2.0);
                }
                ui.horizontal(|ui| {
                    if theme::switch(ui, r.on, r.enabled, &format!("{}, master switch", r.master.label())) {
                        flip = Some((r.master, !r.on));
                    }
                    ui.vertical(|ui| {
                        ui.label(theme::bold(r.master.label(), 15.5));
                        let t = if r.reason { RichText::new(&r.text).color(theme::ORANGE).size(13.0) } else { theme::muted(&r.text) };
                        ui.label(t);
                    });
                });
            }
        });
        if let Some((m, on)) = flip {
            self.mods_ui.store.set_master(m, on);
            if let Err(e) = self.mods_ui.store.save() {
                self.toast(format!("The mod list could not be saved: {e}"), true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_lines_read_well() {
        assert_eq!(status_text(Master::Textures, true, 0, 0), "No texture packs yet. Add one below.");
        assert_eq!(status_text(Master::Textures, true, 3, 1), "1 of 3 packs is switched on below.");
        assert_eq!(status_text(Master::Textures, true, 3, 2), "2 of 3 packs are switched on below.");
        assert!(status_text(Master::Textures, false, 3, 2).starts_with("Off: the game loads no packs."));
        assert!(status_text(Master::Mods, true, 0, 0).contains("No script mods yet"));
        assert!(status_text(Master::Mods, false, 2, 2).starts_with("Off: the game loads no mods."));
    }
}
