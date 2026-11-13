//! Discoverable language packs and per-player preferences. No PS2 language index is invented.
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

pub(crate) const ITEM: u32 = 0xffff_ffed;
static NOTICE: AtomicBool = AtomicBool::new(false);
static PREF_LOCK: Mutex<()> = Mutex::new(());
static SELECTED: OnceLock<Mutex<String>> = OnceLock::new();

pub(crate) use rac_engine::localization::{active_pack, packs};
pub(crate) fn install_root() -> Option<PathBuf> {
    rac_engine::install_root().ok()
}
fn value<'a>(s: &'a str, key: &str) -> Option<&'a str> {
    s.lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.trim())
}
pub(crate) fn preferences_path() -> PathBuf {
    std::env::var_os("RAC_PLAYER_DIR")
        .map(PathBuf::from)
        .or_else(|| install_root().map(|p| p.join("player")))
        .unwrap_or_else(|| PathBuf::from("player"))
        .join("localization.ini")
}
fn prefs() -> String {
    std::fs::read_to_string(preferences_path()).unwrap_or_default()
}
fn set_pref(key: &str, v: &str) -> std::io::Result<()> {
    let _guard = PREF_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let old = prefs();
    let mut lines: Vec<String> = old
        .lines()
        .filter(|l| l.split_once('=').is_none_or(|(k, _)| k.trim() != key))
        .map(str::to_string)
        .collect();
    lines.push(format!("{key}={v}"));
    let path = preferences_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?
    }
    std::fs::write(path, lines.join("\n") + "\n")
}
fn selected_lock() -> &'static Mutex<String> {
    SELECTED.get_or_init(|| {
        Mutex::new(
            active_pack()
                .map(|p| p.id.clone())
                .unwrap_or_else(|| "original".into()),
        )
    })
}
fn selected() -> String {
    selected_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}
fn activate(mod_id: Option<&str>) -> std::io::Result<()> {
    let Some(root) = crate::texpack::mods::mods_root() else {
        return Ok(());
    };
    let mut store = crate::texpack::store::Store::load(&root);
    for p in packs() {
        store.set_mod(&p.mod_id, Some(p.mod_id.as_str()) == mod_id)
    }
    if let Some(id) = mod_id {
        if let Some(master) = store.master_of(id) {
            store.masters.set(master, true)
        }
    }
    store.save()
}
pub(crate) fn configured_base() -> String {
    if let Some(p) = active_pack() {
        return p.base.clone();
    }
    String::new()
}
pub(crate) fn has_notice() -> bool {
    NOTICE.load(Ordering::Relaxed)
}
pub(crate) fn notice_text() -> String {
    "Language selection saved. Restart the game to apply it.\nVolba jazyka je uložena. Změna se projeví po restartu hry.".into()
}
pub(crate) fn answer_notice(bit: u16) -> bool {
    if !has_notice() {
        return false;
    }
    if [0x40, 0x20, 0x10].contains(&bit) {
        NOTICE.store(false, Ordering::Relaxed);
    }
    true
}
pub(crate) fn choose_pack(row: usize) {
    if let Some(p) = packs().get(row) {
        match activate(Some(&p.mod_id)).and_then(|_| set_pref("locale", &p.id)) {
            Ok(()) => {
                *selected_lock().lock().unwrap_or_else(|e| e.into_inner()) = p.id.clone();
                let _ = set_pref("base", &p.base);
                NOTICE.store(true, Ordering::Relaxed);
                eprintln!("locale: selected {} for next start", p.id)
            }
            Err(e) => eprintln!("locale: could not save selection: {e}"),
        }
    }
}
pub(crate) fn choose_builtin(row: usize) {
    if let Some(base) = ["en", "fr", "de", "es", "it"].get(row) {
        if activate(None)
            .and_then(|_| set_pref("locale", "original"))
            .is_ok()
        {
            *selected_lock().lock().unwrap_or_else(|e| e.into_inner()) = "original".into();
            let _ = set_pref("base", base);
            NOTICE.store(true, Ordering::Relaxed);
        }
    }
}
pub(crate) fn display_name(row: usize) -> Option<String> {
    let p = packs().get(row)?;
    let suffix = if selected() == p.id { "  *" } else { "" };
    Some(format!("{}{suffix}", p.name))
}

pub(crate) fn always_subtitles() -> bool {
    active_pack().is_some_and(|pack| pack.flag("force_subtitles"))
}

#[derive(Default)]
struct SettingsState {
    initialized: bool,
    machine: usize,
    context: u64,
    last: bool,
    wanted: bool,
}
pub(crate) fn frame_preferences(ram: &mut [u8], field: u32, context: u64) {
    if active_pack().is_none()
        || super::super::retail::disc::usa()
        || field < 100
        || ram.len() <= 0x15ee40
    {
        return;
    }
    static ST: Mutex<Option<SettingsState>> = Mutex::new(None);
    let Ok(mut g) = ST.lock() else { return };
    let s = g.get_or_insert_with(SettingsState::default);
    let force = always_subtitles();
    let machine = ram.as_ptr() as usize;
    if !s.initialized || machine != s.machine {
        let p = prefs();
        s.wanted = value(&p, "subtitles").map_or(true, |v| v == "on");
        s.last = s.wanted;
        s.initialized = true;
        s.machine = machine;
        s.context = context;
        ram[0x15ee40] = (force || s.wanted) as u8;
        let base = configured_base();
        if !base.is_empty() {
            let word = match base.as_str() {
                "fr" => 2u32,
                "de" => 3,
                "es" => 4,
                "it" => 5,
                _ => 0,
            };
            ram[0x15ed88..0x15ed8c].copy_from_slice(&word.to_le_bytes());
        }
        if value(&p, "subtitles").is_none() {
            let _ = set_pref("subtitles", if s.wanted { "on" } else { "off" });
        }
        eprintln!(
            "locale: restored subtitles {}",
            if s.wanted { "on" } else { "off" }
        );
        return;
    }
    if s.context != context {
        s.context = context;
        ram[0x15ee40] = (force || s.wanted) as u8;
        s.last = s.wanted;
        return;
    }
    if force {
        ram[0x15ee40] = 1;
        return;
    }
    let now = ram[0x15ee40] != 0;
    if now != s.last {
        s.last = now;
        s.wanted = now;
        if let Err(e) = set_pref("subtitles", if now { "on" } else { "off" }) {
            eprintln!("locale: could not save subtitles: {e}")
        } else {
            eprintln!("locale: saved subtitles {}", if now { "on" } else { "off" });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_parser() {
        assert_eq!(
            value("# x\nlocale = cs\nsubtitles=on\n", "locale"),
            Some("cs")
        );
    }
}
