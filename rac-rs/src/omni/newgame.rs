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

use super::Omni;
use ee_oracle::cpu::Cpu;
use ee_oracle::hs_reloc::{Reloc, RelocHand};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

const NG_PLUS_SPEC: &str = "reloc omni_ngplus entry 0x272ee0 len 0x2fc\nmhash 0x40e1fbca3f867b95\nmask 0x50 0x3ffffff\nmask 0x6c 0x3ffffff\n\
mask 0x80 0x3ffffff\nmask 0x94 0x3ffffff\nmask 0xb0 0x3ffffff\nmask 0xc8 0x3ffffff\nmask 0xdc 0x3ffffff\nmask 0x110 0x3ffffff\nmask 0x120 0x3ffffff\n\
mask 0x130 0xffff\nmask 0x138 0xffff\nmask 0x174 0x3ffffff\nmask 0x190 0x3ffffff\nmask 0x1e4 0x3ffffff\nmask 0x1fc 0x3ffffff\nmask 0x298 0xffff\n\
mask 0x2a4 0x3ffffff\n";

/// New Game+ was entered (set by the hand, taken once per field by `take`). New Game from the title never sets it: `FrontWatcher` tells.
static PENDING: AtomicBool = AtomicBool::new(false);

/// New Game+ was entered since the last `take`.
pub fn take() -> bool {
	PENDING.swap(false, Ordering::Relaxed)
}

fn on_ng_plus(_c: &mut Cpu) -> bool {
	PENDING.store(true, Ordering::Relaxed);
	false
}

fn ng_plus_reloc() -> &'static Reloc {
	static R: OnceLock<Reloc> = OnceLock::new();
	R.get_or_init(|| Reloc::parse(NG_PLUS_SPEC).expect("the New Game+ spec parses"))
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

/// Tells a New Game from the title: the front end was left with level word 0 and nothing was loaded from a card since it began.
#[derive(Debug, Default)]
pub struct FrontWatcher {
	front: bool,
	loaded: bool,
}

impl FrontWatcher {
	/// `front`: the front end runs now; `level_word`: 0x15ED84; `loaded`: a `save_loaded` event came in this field. True: New Game.
	pub fn step(&mut self, front: bool, level_word: i32, loaded: bool) -> bool {
		if front && !self.front {
			self.loaded = false;
		}
		self.loaded |= loaded;
		let left = self.front && !front;
		self.front = front;
		left && !self.loaded && level_word == 0
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_ng_plus_spec_parses_with_its_length() {
		assert_eq!(ng_plus_reloc().len, 0x2fc);
	}

	#[test]
	fn take_returns_the_flag_once() {
		PENDING.store(false, Ordering::Relaxed);
		assert!(!take());
		let mut c = Cpu::new(ee_oracle::mem::Mem::new());
		assert!(!on_ng_plus(&mut c));
		assert!(take());
		assert!(!take());
	}

	#[test]
	fn leaving_the_title_without_a_load_is_a_new_game() {
		let mut f = FrontWatcher::default();
		assert!(!f.step(false, -1, false));
		assert!(!f.step(true, 0, false));
		assert!(!f.step(true, 0, false));
		assert!(f.step(false, 0, false));
		assert!(!f.step(false, 0, false));
	}

	#[test]
	fn leaving_the_title_after_a_card_load_is_not() {
		let mut f = FrontWatcher::default();
		f.step(true, 0, false);
		f.step(true, 0, true);
		assert!(!f.step(false, 3, false));
		// The next trip to the title starts over.
		f.step(true, 3, false);
		assert!(f.step(false, 0, false));
	}

	#[test]
	fn a_load_in_the_field_the_title_is_left_counts_and_a_level_other_than_veldin_does_not() {
		let mut f = FrontWatcher::default();
		f.step(true, 0, false);
		assert!(!f.step(false, 0, true));
		f.step(true, 5, false);
		assert!(!f.step(false, 5, false));
	}
}
