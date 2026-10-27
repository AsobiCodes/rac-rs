//! The settings panel of a script mod on the Mods page: preset buttons, one row per setting from the mod's omni-mod.json (a switch, a
//! segmented control or list, a slider or number field, a text field, "press a key" buttons), "Reset to defaults". The panel only
//! returns what the player changed; the page writes it to `mods\settings.json` (`mods::settings::SettingsFile`).
use super::theme;
use crate::mods::keys::{self, Capture};
use crate::mods::script::ScriptMod;
use crate::mods::settings::{self, Kind, Lit, SettingDef};
use egui::{Color32, RichText, Ui};
use serde_json::{Map, Value as Json};
use std::collections::HashMap;

/// A change to write: `Some(value)` sets the key, `None` removes it (back to the default).
pub type Changes = Vec<(String, Option<Json>)>;

/// What the panel remembers between frames (per page, not saved).
#[derive(Default)]
pub struct PanelState {
    /// Slider / number values while the pointer is still dragging (written when it is let go).
    pub drag: HashMap<String, f64>,
    /// Text being typed (written when the field loses focus).
    pub text: HashMap<String, String>,
    /// The keybind button that waits for a key: (mod id, setting key, slot).
    pub listening: Option<(String, String, usize)>,
}

fn all_changes(effect: Vec<(String, Json)>) -> Changes {
    effect.into_iter().map(|(k, v)| (k, Some(v))).collect()
}

/// The whole panel. `given` = the stored values of this mod. `locked` = the file cannot be written (greyed).
pub fn panel(ui: &mut Ui, m: &ScriptMod, given: &Map<String, Json>, st: &mut PanelState, locked: bool) -> Changes {
    let cur = settings::resolve(&m.settings, given);
    let mut out: Changes = Vec::new();
    ui.add_enabled_ui(!locked, |ui| {
        presets_row(ui, m, &cur, &mut out);
        ui.add_space(4.0);
        for d in m.settings.iter().filter(|d| !d.advanced) {
            setting_row(ui, m, d, &cur, st, &mut out);
        }
        if m.settings.iter().any(|d| d.advanced) {
            egui::CollapsingHeader::new(RichText::new("Advanced").size(14.0).color(theme::MUTED)).id_salt((&m.id, "advanced")).show(ui, |ui| {
                for d in m.settings.iter().filter(|d| d.advanced) {
                    setting_row(ui, m, d, &cur, st, &mut out);
                }
            });
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if theme::button_enabled(ui, "Reset to defaults", !settings::all_default(&m.settings, &cur)) {
                out = m.settings.iter().map(|d| (d.key.clone(), None)).collect();
            }
            ui.label(theme::muted("Changes apply the next time the game starts."));
        });
    });
    if !out.is_empty() {
        st.drag.clear();
        st.text.clear();
        st.listening = None;
    }
    out
}

/// "Default" and the mod's presets as one row of buttons; the one whose values the settings have is lit.
fn presets_row(ui: &mut Ui, m: &ScriptMod, cur: &[Json], out: &mut Changes) {
    let has_default_preset = m.presets.iter().any(|p| p.id == "default");
    let mut labels: Vec<&str> = Vec::new();
    if !has_default_preset {
        labels.push("Default");
    }
    labels.extend(m.presets.iter().map(|p| p.name.as_str()));
    if labels.len() < 2 && m.presets.is_empty() {
        return; // a mod without presets: the Reset button below is enough
    }
    let lit = settings::lit(&m.settings, &m.presets, cur);
    let offset = usize::from(!has_default_preset);
    let selected = lit.map(|l| match l {
        Lit::Default => 0,
        Lit::Preset(i) => i + offset,
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(theme::bold("Presets", 14.5));
        let name = format!("{} preset", m.name);
        let clicked = if theme::segmented_width(ui, &labels) <= ui.available_width() {
            theme::segmented(ui, &m.id, &name, &labels, selected)
        } else {
            wrapped_buttons(ui, &labels, selected)
        };
        if let Some(i) = clicked {
            *out = if i < offset { all_changes(settings::default_effect(&m.settings)) } else { all_changes(settings::preset_effect(&m.settings, &m.presets, &m.presets[i - offset])) };
        }
    });
    if selected.is_none() {
        ui.label(theme::muted("Custom: your own mix of values."));
    }
}

fn wrapped_buttons(ui: &mut Ui, labels: &[&str], selected: Option<usize>) -> Option<usize> {
    let mut clicked = None;
    for (i, l) in labels.iter().enumerate() {
        let on = selected == Some(i);
        let b = egui::Button::new(RichText::new(*l).color(if on { theme::ON_ORANGE } else { theme::TEXT })).fill(if on { theme::ORANGE } else { theme::CARD_HI });
        if ui.add(b).clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// The room a keybind's buttons need: every slot, "Clear" and the gaps.
fn keybind_width(kind: &Kind) -> f32 {
    match kind {
        Kind::Keybind { binds, .. } => *binds as f32 * 128.0 + 60.0,
        _ => 0.0,
    }
}

fn setting_row(ui: &mut Ui, m: &ScriptMod, d: &SettingDef, cur: &[Json], st: &mut PanelState, out: &mut Changes) {
    let Some(idx) = m.settings.iter().position(|x| x.key == d.key) else { return };
    let value = &cur[idx];
    let conflicts = settings::bind_conflicts(&m.settings, cur, &d.key);
    let avail = ui.available_width();
    // a keybind's buttons need their room: when the label would be squeezed, the buttons go under it
    let need = keybind_width(&d.kind);
    let stacked = need > 0.0 && avail - need < 220.0;
    let left_w = if stacked { avail } else { (avail * 0.5).clamp(180.0, 430.0).min(avail - need).max(150.0) };
    let label_col = |ui: &mut Ui| {
        ui.allocate_ui_with_layout(egui::vec2(left_w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            let l = ui.label(theme::bold(&d.name, 14.5));
            if !d.description.is_empty() {
                ui.label(theme::muted(&d.description).size(12.5));
            }
            if !conflicts.is_empty() {
                ui.label(RichText::new(format!("Also used by {}: both react to the same input.", conflicts.join(", "))).color(theme::ORANGE).size(12.5));
            }
            l
        })
        .inner
    };
    let mut controls = |ui: &mut Ui, label: &egui::Response| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            let changed = match &d.kind {
                Kind::Bool => bool_control(ui, d, value),
                Kind::Choice { choices } => choice_control(ui, m, d, choices, value, ui.available_width()),
                Kind::Int { .. } | Kind::Float { .. } => number_control(ui, m, d, value, st, label),
                Kind::Str { max_length } => text_control(ui, m, d, *max_length, value, st),
                Kind::Keybind { binds, .. } => keybind_control(ui, m, d, *binds, value, st),
            };
            if let Some(v) = changed {
                out.push((d.key.clone(), Some(v)));
            }
        });
    };
    if stacked {
        ui.vertical(|ui| {
            let label = label_col(ui);
            controls(ui, &label);
        });
    } else {
        ui.horizontal_top(|ui| {
            let label = label_col(ui);
            controls(ui, &label);
        });
    }
    ui.add_space(4.0);
}

fn bool_control(ui: &mut Ui, d: &SettingDef, value: &Json) -> Option<Json> {
    let on = value.as_bool().unwrap_or(false);
    theme::switch(ui, on, true, &format!("{}, switch", d.name)).then(|| Json::Bool(!on))
}

fn choice_control(ui: &mut Ui, m: &ScriptMod, d: &SettingDef, choices: &[settings::Choice], value: &Json, room: f32) -> Option<Json> {
    let labels: Vec<&str> = choices.iter().map(|c| c.name.as_str()).collect();
    let sel = choices.iter().position(|c| Some(c.value.as_str()) == value.as_str());
    if labels.len() <= 4 && theme::segmented_width(ui, &labels) <= room {
        return theme::segmented(ui, &format!("{}.{}", m.id, d.key), &d.name, &labels, sel).map(|i| Json::from(choices[i].value.clone()));
    }
    let mut picked = None;
    let combo = egui::ComboBox::from_id_salt((&m.id, &d.key)).width(room.min(250.0)).selected_text(sel.map(|i| labels[i]).unwrap_or("")).show_ui(ui, |ui| {
        for (i, c) in choices.iter().enumerate() {
            if ui.selectable_label(sel == Some(i), &c.name).clicked() {
                picked = Some(Json::from(c.value.clone()));
            }
        }
    });
    combo.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, d.name.clone()));
    picked
}

/// The launcher's look for a slider: a round handle with an orange ring, the filled part orange, the rest of the rail dark.
fn slider_style(ui: &mut Ui) {
    let v = ui.visuals_mut();
    v.handle_shape = egui::style::HandleShape::Circle;
    v.slider_trailing_fill = true;
    v.selection.bg_fill = theme::ORANGE;
    v.widgets.inactive.bg_fill = Color32::from_rgb(0x34, 0x3E, 0x50);
    v.widgets.hovered.bg_fill = Color32::from_rgb(0x44, 0x50, 0x66);
    v.widgets.active.bg_fill = Color32::from_rgb(0x44, 0x50, 0x66);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active] {
        w.fg_stroke = egui::Stroke::new(2.0, theme::ORANGE);
    }
    ui.spacing_mut().slider_width = 150.0;
}

fn number_control(ui: &mut Ui, m: &ScriptMod, d: &SettingDef, value: &Json, st: &mut PanelState, label: &egui::Response) -> Option<Json> {
    let key = format!("{}.{}", m.id, d.key);
    let shown = st.drag.get(&key).copied().or_else(|| value.as_f64()).unwrap_or(0.0);
    let mut v = shown;
    let resp = match &d.kind {
        Kind::Int { min: Some(lo), max: Some(hi) } => ui.scope(|ui| {
            slider_style(ui);
            ui.add(egui::Slider::new(&mut v, *lo as f64..=*hi as f64).integer())
        })
        .inner,
        Kind::Int { min, max } => {
            let mut dv = egui::DragValue::new(&mut v).speed(1.0).max_decimals(0);
            if let (Some(lo), Some(hi)) = (min, max) {
                dv = dv.range(*lo as f64..=*hi as f64);
            } else if let Some(lo) = min {
                dv = dv.range(*lo as f64..=f64::MAX);
            } else if let Some(hi) = max {
                dv = dv.range(f64::MIN..=*hi as f64);
            }
            ui.add(dv)
        }
        Kind::Float { min: Some(lo), max: Some(hi), step } => {
            let mut s = egui::Slider::new(&mut v, *lo..=*hi).max_decimals(4);
            if let Some(step) = step {
                s = s.step_by(*step);
            }
            ui.scope(|ui| {
                slider_style(ui);
                ui.add(s)
            })
            .inner
        }
        Kind::Float { min, max, .. } => ui.add(egui::DragValue::new(&mut v).speed(0.05).range(min.unwrap_or(f64::MIN)..=max.unwrap_or(f64::MAX))),
        _ => return None,
    };
    let resp = resp.labelled_by(label.id);
    if resp.changed() {
        st.drag.insert(key.clone(), v);
    }
    if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
        let v = settings::snap(d, st.drag.remove(&key).unwrap_or(v));
        if !settings::same(&Json::from(v), value) {
            return Some(match d.kind {
                Kind::Int { .. } => Json::from(v.round() as i64),
                _ => Json::from(v),
            });
        }
    }
    None
}

fn text_control(ui: &mut Ui, m: &ScriptMod, d: &SettingDef, max_length: Option<usize>, value: &Json, st: &mut PanelState) -> Option<Json> {
    let key = format!("{}.{}", m.id, d.key);
    let id = ui.make_persistent_id((&m.id, &d.key, "text"));
    let focused = ui.memory(|mem| mem.has_focus(id));
    let mut buf = if focused { st.text.get(&key).cloned() } else { None }.unwrap_or_else(|| value.as_str().unwrap_or("").to_string());
    let mut edit = egui::TextEdit::singleline(&mut buf).id(id).desired_width(220.0);
    if let Some(n) = max_length {
        edit = edit.char_limit(n);
    }
    let resp = ui.add(edit);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, d.name.clone()));
    if resp.changed() {
        st.text.insert(key.clone(), buf.clone());
    }
    if resp.lost_focus() {
        st.text.remove(&key);
        if value.as_str() != Some(buf.as_str()) {
            return Some(Json::from(buf));
        }
    }
    None
}

fn keybind_control(ui: &mut Ui, m: &ScriptMod, d: &SettingDef, binds: usize, value: &Json, st: &mut PanelState) -> Option<Json> {
    let text = value.as_str().unwrap_or("");
    let names: Vec<String> = keys::bind_names(text).into_iter().map(str::to_string).collect();
    let mut result = None;
    // right_to_left: the last slot is drawn first, so draw them in reverse to read left to right
    if !names.is_empty() && theme::link(ui, "Clear") {
        result = Some(Json::from(""));
    }
    for slot in (0..binds).rev() {
        let mine = matches!(&st.listening, Some((a, b, s)) if *a == m.id && *b == d.key && *s == slot);
        let label = if mine {
            "Press a key or button...".to_string()
        } else {
            names.get(slot).map(|n| keys::friendly(n)).unwrap_or_else(|| "Not set".to_string())
        };
        let color = if mine { theme::ON_ORANGE } else if names.get(slot).is_some() { theme::TEXT } else { theme::MUTED };
        let b = egui::Button::new(RichText::new(&label).color(color)).min_size(egui::vec2(120.0, 28.0)).fill(if mine { theme::ORANGE } else { theme::CARD_HI });
        let resp = ui.add(b);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{}, input {}: {}", d.name, slot + 1, label)));
        if resp.clicked() {
            st.listening = if mine { None } else { Some((m.id.clone(), d.key.clone(), slot)) };
        }
        if mine {
            ui.ctx().request_repaint();
            let events = ui.input(|i| i.events.clone());
            match keys::capture(&events) {
                Some(Capture::Cancel) => st.listening = None,
                Some(Capture::Input(n)) => {
                    let mut list = names.clone();
                    let at = slot.min(list.len());
                    if at < list.len() {
                        list[at] = n.clone();
                    } else {
                        list.push(n.clone());
                    }
                    // the same input in the other slot would be a double: it moves here
                    let mut i = 0;
                    list.retain(|x| {
                        i += 1;
                        i - 1 == at || *x != n
                    });
                    list.truncate(binds);
                    result = Some(Json::from(list.join(",")));
                    st.listening = None;
                }
                None => {}
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::script::{self, Parsed};
    use egui_kittest::kittest::Queryable;
    use egui_kittest::Harness;

    const ARSENAL: &str = include_str!("../../tests/data/arsenal-mastery.omni-mod.json");

    fn arsenal() -> ScriptMod {
        match script::parse(ARSENAL) {
            Parsed::Mod(m) => *m,
            other => panic!("{other:?}"),
        }
    }

    fn harness_for(m: ScriptMod, given: Map<String, Json>) -> Harness<'static, (ScriptMod, Map<String, Json>, PanelState, Vec<Changes>)> {
        Harness::builder().with_size(egui::vec2(900.0, 900.0)).with_context_setup(|c| theme::apply(c)).build_ui_state(
            |ui, (m, given, st, got): &mut (ScriptMod, Map<String, Json>, PanelState, Vec<Changes>)| {
                let c = panel(ui, m, given, st, false);
                if !c.is_empty() {
                    got.push(c);
                }
            },
            (m, given, PanelState::default(), Vec::new()),
        )
    }

    #[test]
    fn a_preset_click_asks_for_every_covered_value() {
        let mut h = harness_for(arsenal(), Map::new());
        h.get_by_label("Arsenal Mastery preset: Survival").click();
        h.run();
        let got = &h.state().3;
        assert_eq!(got.len(), 1);
        let c = &got[0];
        let v = |k: &str| c.iter().find(|(key, _)| key == k).and_then(|(_, v)| v.clone());
        assert_eq!(v("rebalance"), Some(Json::from("hard")));
        assert_eq!(v("ammo_mode"), Some(Json::from("survival")));
        assert_eq!(v("level_cap"), Some(Json::from("uya")));
        assert!(v("start_level").is_some(), "start_level is named by the presets");
    }

    #[test]
    fn a_switch_and_a_choice_ask_for_their_value() {
        let mut h = harness_for(arsenal(), Map::new());
        h.get_by_label("Weapon power from levels, switch").click();
        h.run();
        h.get_by_label("Enemy rebalance: Hard").click();
        h.run();
        let got = &h.state().3;
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[0], vec![("level_power".to_string(), Some(Json::Bool(false)))]);
        assert_eq!(got[1], vec![("rebalance".to_string(), Some(Json::from("hard")))]);
    }

    #[test]
    fn reset_removes_every_declared_key_and_is_greyed_at_the_defaults() {
        let mut h = harness_for(arsenal(), Map::new());
        h.get_by_label("Reset to defaults").click();
        h.run();
        assert!(h.state().3.is_empty(), "nothing to reset at the defaults");
        let given = serde_json::json!({ "xp_rate": 2.5, "perks": false, "unknown": 1 }).as_object().unwrap().clone();
        let mut h = harness_for(arsenal(), given);
        h.get_by_label("Reset to defaults").click();
        h.run();
        let c = &h.state().3[0];
        assert_eq!(c.len(), 13);
        assert!(c.iter().all(|(_, v)| v.is_none()));
        assert!(c.iter().all(|(k, _)| k != "unknown"), "keys the mod does not declare are not touched");
    }

    #[test]
    fn the_listening_keybind_takes_the_next_key_and_escape_cancels() {
        let defs = r#"{"format":2,"id":"kb","name":"KB","version":"1.0.0","type":"script","games":{"rac1":["pal"]},
          "settings":[{"key":"jump","name":"Jump","type":"keybind","default":"Space","group":"g"},{"key":"fire","name":"Fire","type":"keybind","default":"KeyF","binds":1,"group":"g"}]}"#;
        let Parsed::Mod(m) = script::parse(defs) else { panic!() };
        let mut h = harness_for(*m, Map::new());
        h.get_by_label("Jump, input 1: Space").click();
        h.step();
        assert!(h.state().2.listening.is_some());
        h.key_press(egui::Key::J);
        h.step();
        let got = &h.state().3;
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0], vec![("jump".to_string(), Some(Json::from("KeyJ")))], "slot 1 replaced");
        assert!(h.state().2.listening.is_none());
        h.get_by_label("Fire, input 1: F").click();
        h.step();
        h.key_press(egui::Key::Escape);
        h.step();
        assert_eq!(h.state().3.len(), 1, "Escape changes nothing");
        assert!(h.state().2.listening.is_none());
    }
}
