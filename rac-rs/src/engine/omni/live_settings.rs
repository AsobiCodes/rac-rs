//! Script-mod settings changed from the game's Mods pages while the game runs (retail/pcmods.rs): the rows ask for the mods the loader
//! has running (`loaded`) and queue a change (`queue`); the next logic tick applies it (`apply`, with the machine in scope as every call
//! into the DLL has), so `omni.setting(key)` returns the new value at once and the mod's `"setting"` callbacks run.
use super::json::{self, Json};
use super::{host, lock, Omni};
use ee_oracle::cpu::Cpu;
use std::sync::Mutex;

/// (mod id, key, new value as JSON text).
static PENDING: Mutex<Vec<(String, String, String)>> = Mutex::new(Vec::new());

/// A mod the loader runs: its settings declarations, each with its current `"value"`.
#[derive(Clone, Debug)]
pub struct Loaded {
	pub id: String,
	pub settings: Vec<Json>,
}

/// The mods that are running now, in load order (empty without a loader, or with a DLL that has no `omni_settings_json`).
pub fn loaded() -> Vec<Loaded> {
	let Some(o) = super::get() else { return Vec::new() };
	let text = {
		let _held = lock::enter();
		o.loader.settings_json()
	};
	let Some(j) = text.and_then(|t| json::parse(&t).ok()) else { return Vec::new() };
	j.get("mods")
		.map_or(&[][..], Json::arr)
		.iter()
		.filter(|m| m.get("loaded").is_some_and(Json::bool))
		.filter_map(|m| {
			let id = m.get("id").and_then(Json::str)?.to_string();
			Some(Loaded { id, settings: m.get("settings").map_or(Vec::new(), |s| s.arr().to_vec()) })
		})
		.collect()
}

/// Asks for setting `key` of mod `id` to become `value` (JSON text) at the next tick.
pub fn queue(id: &str, key: &str, value: &str) {
	if super::get().is_some() {
		PENDING.lock().unwrap_or_else(|e| e.into_inner()).push((id.to_string(), key.to_string(), value.to_string()));
	}
}

/// Applies the queued changes (worker thread, `omni::tick`).
pub(super) fn apply(o: &Omni, c: &mut Cpu) {
	let list = std::mem::take(&mut *PENDING.lock().unwrap_or_else(|e| e.into_inner()));
	for (id, key, value) in list {
		let _held = lock::enter();
		match host::scope(c, || o.loader.set_setting(&id, &key, &value)) {
			Ok(()) => super::logq::line(&format!("omni: setting {id}.{key} = {value}")),
			Err(e) => super::logq::line(&format!("omni: setting {id}.{key} = {value} refused: {e}")),
		}
	}
}
