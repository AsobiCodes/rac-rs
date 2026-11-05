//! The `new_game` event (ABI 1.2).
//!
//! - New Game from the title: the title's list item does NOT call the overlay NewGame function. It creates the save file, runs the boot
//!   function FUN_00208e40 (reload the default state) and FUN_0022dc98 (leave the title loop). That boot code belongs to the compiled front
//!   end (crates l19 / l20), where registered hands never bind (oracle `Table::build`), so no hand can reach it, so the event comes from `FrontWatcher`: the front end (title and its menus, `events::FRONT`)
//!   was left while the level word is 0 and no `save_loaded` event came since the front end began.
//! - New Game+ (challenge mode, Veldin 0x2842e8, 764 bytes, keeps the selected state, `++0x15ee20`): a host-internal relocatable hand that only
//!   WATCHES, found by its code in every level and on the USA disc like a mod hook point (`hooks.rs`; spec from tools/make_spec.py
//!   `--after func_00227B00:3 --len 764`). UNPROVEN in a run (no New Game+ run was made). It sets a flag and returns false, so the retail
//!   function runs unchanged; `events::send` hands the flag to the mods after the field. The hand exists only when at least one mod is loaded
//!   and the DLL has `omni_event` (`install`); with no mods nothing is registered and the machine and its timing are those of a build without
//!   this file.
//!
//! The level of the New Game rule and the New Game+ spec are the game's (`Game::new_game_level`, `Game::ng_plus`; R&C1's: rac1/omni_bind.rs).

use super::Omni;
use ee_oracle::cpu::Cpu;
use ee_oracle::hs_reloc::{Reloc, RelocHand};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

/// New Game+ was entered (set by the hand, taken once per field by `take`). New Game from the title never sets it: `FrontWatcher` tells.
pub static PENDING: AtomicBool = AtomicBool::new(false);

/// New Game+ was entered since the last `take`.
pub fn take() -> bool {
	PENDING.swap(false, Ordering::Relaxed)
}

pub fn on_ng_plus(_c: &mut Cpu) -> bool {
	PENDING.store(true, Ordering::Relaxed);
	false
}

pub fn ng_plus_reloc() -> &'static Reloc {
	static R: OnceLock<Reloc> = OnceLock::new();
	R.get_or_init(|| Reloc::parse(super::game::get().ng_plus).expect("the New Game+ spec parses"))
}

/// Registers the hand (before the recompiled table is built, i.e. from `omni::init`) when events can reach a loaded mod.
pub fn install(o: &Omni) {
	if !super::events::should_send(o.mods, o.loader.has_events()) {
		return;
	}
	static HAND: RelocHand = RelocHand { name: "omni_ngplus", reloc: ng_plus_reloc, est: ee_oracle::hs::EXACT, f: on_ng_plus };
	ee_oracle::rec::register_reloc_hand(&HAND);
	super::hooks::allow_reloc_hands();
	eprintln!("omni: new_game watch installed (relocatable hand at the retail New Game+ function)");
}

/// Tells a New Game from the title: the front end was left with the level word at the game's first level and nothing was loaded from a
/// card since it began.
#[derive(Debug, Default)]
pub struct FrontWatcher {
	front: bool,
	loaded: bool,
}

impl FrontWatcher {
	/// `front`: the front end runs now; `level_word`: the game's level word; `loaded`: a `save_loaded` event came in this field. True: New Game.
	pub fn step(&mut self, front: bool, level_word: i32, loaded: bool) -> bool {
		if front && !self.front {
			self.loaded = false;
		}
		self.loaded |= loaded;
		let left = self.front && !front;
		self.front = front;
		left && !self.loaded && level_word == super::game::get().new_game_level
	}
}

