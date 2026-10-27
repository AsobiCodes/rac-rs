//! Keybind values of script mods (the format's settings): up to `binds` input names from the SDK's key table, separated by commas.
//! The table is the names of the format's `keys.h` (mod interface 1.3), the same as the OmniLoader's.

pub const BIND_NAMES: &[&str] = &[
    "KeyA", "KeyB", "KeyC", "KeyD", "KeyE", "KeyF", "KeyG", "KeyH", "KeyI", "KeyJ", "KeyK", "KeyL", "KeyM",
    "KeyN", "KeyO", "KeyP", "KeyQ", "KeyR", "KeyS", "KeyT", "KeyU", "KeyV", "KeyW", "KeyX", "KeyY", "KeyZ",
    "Digit1", "Digit2", "Digit3", "Digit4", "Digit5", "Digit6", "Digit7", "Digit8", "Digit9", "Digit0",
    "Enter", "Escape", "Backspace", "Tab", "Space", "Minus", "Equal", "BracketLeft", "BracketRight",
    "Backslash", "IntlHash", "Semicolon", "Quote", "Backquote", "Comma", "Period", "Slash", "CapsLock", "F1",
    "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "PrintScreen", "ScrollLock", "Pause",
    "Insert", "Home", "PageUp", "Delete", "End", "PageDown", "ArrowRight", "ArrowLeft", "ArrowDown",
    "ArrowUp", "NumLock", "NumpadDivide", "NumpadMultiply", "NumpadSubtract", "NumpadAdd", "NumpadEnter",
    "Numpad1", "Numpad2", "Numpad3", "Numpad4", "Numpad5", "Numpad6", "Numpad7", "Numpad8", "Numpad9",
    "Numpad0", "NumpadDecimal", "IntlBackslash", "ContextMenu", "NumpadEqual", "ControlLeft", "ShiftLeft",
    "AltLeft", "MetaLeft", "ControlRight", "ShiftRight", "AltRight", "MetaRight", "MouseLeft", "MouseRight",
    "MouseMiddle", "MouseBack", "MouseForward", "WheelUp", "WheelDown", "WheelLeft", "WheelRight",
];

pub fn is_bind_name(name: &str) -> bool {
    BIND_NAMES.contains(&name)
}

/// The inputs of a keybind value (`"KeyF,MouseMiddle"` -> `["KeyF", "MouseMiddle"]`; commas or spaces separate).
pub fn bind_names(value: &str) -> Vec<&str> {
    value.split(|c: char| c == ',' || c.is_whitespace()).filter(|n| !n.is_empty()).collect()
}

/// Why a keybind value is not one (names of the table, none twice, at most `most`), or `None`. `""` binds nothing and is valid.
pub fn bind_problem(value: &str, most: usize) -> Option<String> {
    let names = bind_names(value);
    let unknown: Vec<&str> = names.iter().copied().filter(|n| !is_bind_name(n)).collect();
    if !unknown.is_empty() {
        return Some(format!("names no key, button or wheel direction: {}", unknown.join(", ")));
    }
    let mut seen = std::collections::BTreeSet::new();
    if !names.iter().all(|n| seen.insert(*n)) {
        return Some("names one input twice".to_string());
    }
    if names.len() > most {
        return Some(format!("has {} binds, more than {}", names.len(), most));
    }
    None
}

/// What a player reads for an input name: `KeyF` -> "F", `Digit1` -> "1", `MouseMiddle` -> "Middle mouse button", `ArrowUp` -> "Up arrow".
pub fn friendly(name: &str) -> String {
    if let Some(k) = name.strip_prefix("Key").filter(|k| k.len() == 1) {
        return k.to_string();
    }
    if let Some(d) = name.strip_prefix("Digit") {
        return d.to_string();
    }
    if let Some(n) = name.strip_prefix("Numpad") {
        let sym = match n {
            "Divide" => "/",
            "Multiply" => "*",
            "Subtract" => "-",
            "Add" => "+",
            "Decimal" => ".",
            "Equal" => "=",
            other => other,
        };
        return format!("Numpad {sym}");
    }
    if let Some(a) = name.strip_prefix("Arrow") {
        return format!("{a} arrow");
    }
    let t = match name {
        "MouseLeft" => "Left mouse button",
        "MouseRight" => "Right mouse button",
        "MouseMiddle" => "Middle mouse button",
        "MouseBack" => "Mouse back button",
        "MouseForward" => "Mouse forward button",
        "WheelUp" => "Wheel up",
        "WheelDown" => "Wheel down",
        "WheelLeft" => "Wheel left",
        "WheelRight" => "Wheel right",
        "ShiftLeft" => "Left Shift",
        "ShiftRight" => "Right Shift",
        "ControlLeft" => "Left Ctrl",
        "ControlRight" => "Right Ctrl",
        "AltLeft" => "Left Alt",
        "AltRight" => "Right Alt",
        "BracketLeft" => "[",
        "BracketRight" => "]",
        "Minus" => "-",
        "Equal" => "=",
        "Comma" => ",",
        "Period" => ".",
        "Slash" => "/",
        "Backslash" => "\\",
        "Semicolon" => ";",
        "Quote" => "'",
        "Backquote" => "`",
        other => other,
    };
    t.to_string()
}

/// The SDK name of an egui key, when the table has it. Modifier keys are not egui keys and cannot be captured.
pub fn name_of_key(key: egui::Key) -> Option<String> {
    use egui::Key as K;
    let fixed = match key {
        K::Num0 => "Digit0",
        K::Num1 => "Digit1",
        K::Num2 => "Digit2",
        K::Num3 => "Digit3",
        K::Num4 => "Digit4",
        K::Num5 => "Digit5",
        K::Num6 => "Digit6",
        K::Num7 => "Digit7",
        K::Num8 => "Digit8",
        K::Num9 => "Digit9",
        K::Minus => "Minus",
        K::Equals | K::Plus => "Equal",
        K::OpenBracket => "BracketLeft",
        K::CloseBracket => "BracketRight",
        K::Backslash => "Backslash",
        K::Semicolon | K::Colon => "Semicolon",
        K::Quote => "Quote",
        K::Backtick => "Backquote",
        K::Comma => "Comma",
        K::Period => "Period",
        K::Slash => "Slash",
        K::ArrowLeft => "ArrowLeft",
        K::ArrowRight => "ArrowRight",
        K::ArrowUp => "ArrowUp",
        K::ArrowDown => "ArrowDown",
        K::Enter => "Enter",
        K::Escape => "Escape",
        K::Tab => "Tab",
        K::Backspace => "Backspace",
        K::Space => "Space",
        K::Insert => "Insert",
        K::Delete => "Delete",
        K::Home => "Home",
        K::End => "End",
        K::PageUp => "PageUp",
        K::PageDown => "PageDown",
        K::F1 => "F1",
        K::F2 => "F2",
        K::F3 => "F3",
        K::F4 => "F4",
        K::F5 => "F5",
        K::F6 => "F6",
        K::F7 => "F7",
        K::F8 => "F8",
        K::F9 => "F9",
        K::F10 => "F10",
        K::F11 => "F11",
        K::F12 => "F12",
        _ => "",
    };
    let s = key.name();
    let name = if !fixed.is_empty() {
        fixed.to_string()
    } else if s.len() == 1 && s.as_bytes()[0].is_ascii_uppercase() {
        format!("Key{s}")
    } else {
        return None;
    };
    is_bind_name(&name).then_some(name)
}

/// What a keybind button that listens got from the events of a frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Capture {
    /// Escape: stop listening, change nothing.
    Cancel,
    Input(String),
}

/// The first input pressed in `events`. The left mouse button is never captured (it is how the button is clicked).
pub fn capture(events: &[egui::Event]) -> Option<Capture> {
    use egui::{Event, PointerButton};
    for ev in events {
        match ev {
            Event::Key { key, physical_key, pressed: true, repeat: false, .. } => {
                let k = physical_key.unwrap_or(*key);
                if k == egui::Key::Escape {
                    return Some(Capture::Cancel);
                }
                if let Some(n) = name_of_key(k).or_else(|| name_of_key(*key)) {
                    return Some(Capture::Input(n));
                }
            }
            Event::PointerButton { button, pressed: true, .. } => {
                let n = match button {
                    PointerButton::Secondary => "MouseRight",
                    PointerButton::Middle => "MouseMiddle",
                    PointerButton::Extra1 => "MouseBack",
                    PointerButton::Extra2 => "MouseForward",
                    PointerButton::Primary => continue,
                };
                return Some(Capture::Input(n.to_string()));
            }
            Event::MouseWheel { delta, .. } if delta.x != 0.0 || delta.y != 0.0 => {
                let n = if delta.y.abs() >= delta.x.abs() {
                    if delta.y > 0.0 {
                        "WheelUp"
                    } else {
                        "WheelDown"
                    }
                } else if delta.x > 0.0 {
                    "WheelLeft"
                } else {
                    "WheelRight"
                };
                return Some(Capture::Input(n.to_string()));
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds() {
        assert!(bind_problem("KeyF,MouseMiddle", 2).is_none());
        assert!(bind_problem("", 1).is_none());
        assert!(bind_problem("KeyF,KeyF", 2).is_some());
        assert!(bind_problem("KeyA,KeyB", 1).is_some());
        assert!(bind_problem("Nope", 2).unwrap().contains("Nope"));
        assert!(is_bind_name("WheelUp") && is_bind_name("Space") && is_bind_name("ShiftLeft"));
        assert_eq!(bind_names("KeyF, MouseMiddle"), vec!["KeyF", "MouseMiddle"]);
    }

    #[test]
    fn names_read_well_and_every_name_has_text() {
        assert_eq!(friendly("KeyF"), "F");
        assert_eq!(friendly("Digit1"), "1");
        assert_eq!(friendly("ArrowUp"), "Up arrow");
        assert_eq!(friendly("MouseMiddle"), "Middle mouse button");
        assert_eq!(friendly("NumpadAdd"), "Numpad +");
        assert_eq!(friendly("Space"), "Space");
        assert!(BIND_NAMES.iter().all(|n| !friendly(n).is_empty()));
    }

    #[test]
    fn egui_keys_map_to_sdk_names() {
        assert_eq!(name_of_key(egui::Key::F).as_deref(), Some("KeyF"));
        assert_eq!(name_of_key(egui::Key::Num3).as_deref(), Some("Digit3"));
        assert_eq!(name_of_key(egui::Key::ArrowLeft).as_deref(), Some("ArrowLeft"));
        assert_eq!(name_of_key(egui::Key::Space).as_deref(), Some("Space"));
        assert_eq!(name_of_key(egui::Key::F5).as_deref(), Some("F5"));
        assert_eq!(name_of_key(egui::Key::Backtick).as_deref(), Some("Backquote"));
        assert_eq!(name_of_key(egui::Key::PageDown).as_deref(), Some("PageDown"));
    }

    #[test]
    fn capture_takes_keys_mouse_buttons_and_the_wheel_but_not_the_left_click() {
        use egui::{Event, Modifiers, PointerButton, Pos2};
        let key = |k| Event::Key { key: k, physical_key: Some(k), pressed: true, repeat: false, modifiers: Modifiers::NONE };
        let btn = |b| Event::PointerButton { pos: Pos2::ZERO, button: b, pressed: true, modifiers: Modifiers::NONE };
        assert_eq!(capture(&[key(egui::Key::G)]), Some(Capture::Input("KeyG".into())));
        assert_eq!(capture(&[key(egui::Key::Escape)]), Some(Capture::Cancel));
        assert_eq!(capture(&[btn(PointerButton::Primary)]), None);
        assert_eq!(capture(&[btn(PointerButton::Primary), btn(PointerButton::Middle)]), Some(Capture::Input("MouseMiddle".into())));
        assert_eq!(capture(&[btn(PointerButton::Extra1)]), Some(Capture::Input("MouseBack".into())));
        let wheel = Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: egui::vec2(0.0, -1.0), phase: egui::TouchPhase::Move, source: egui::MouseWheelSource::Wheel, modifiers: Modifiers::NONE };
        assert_eq!(capture(&[wheel]), Some(Capture::Input("WheelDown".into())));
        let up = Event::Key { key: egui::Key::G, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE };
        assert_eq!(capture(&[up]), None, "a release is not a bind");
    }
}
