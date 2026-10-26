//! Memory-card events for the mods (ABI 1.2 `saving`, `save_loaded`): read from the game's card state machine, never written.
//!
//! The game's card state machine (decomp/notes/savegame.md "Novalis card state machine") keeps its state in boot data, the same
//! words on both discs: `0x13d364` the state, `0x13d36c` the queued command (-1 none, 0xf autosave, 0x13 whole save), `0x13d370` the target
//! slot of that command (consumed and set to -1 when the state machine takes the command), `0x13d2a4` the CURRENT SAVE SLOT the game plays in (the `saveN.bin` its saves go to; 0..4; set by Load Game / New Game / the title's slot choice).
//! (`0x13d350` is NOT a slot: it is the memory-card port index, 0 on every run seen, which is why C4/C5 read 0 while save4 / save1 were written; corrected in C6.)
//! States: 0x10 autosave write (open, seek, write block A, write the source planet's block B, close), 0x13 / 0x14 whole-image write,
//! 0xd / 0xe full load (header, block A, 20 blocks B), 0x17 summary scan of all slots (NOT a load).
//!
//! Once per field (`events::send`, after the EE ran the field and before the mods' `on_frame` and before `Saver::after_field` writes card.bin):
//! - `saving(slot)` when the state enters 0x10 / 0x13 / 0x14 (confirmed on a real manual save: 0x13, then 0x14 with the file open; the event
//!   of a manual save comes with the file open, before any byte is written). The game's own file writes follow in the next fields, so the mods change their
//!   store in time; the loader writes its slot file inside the event.
//! - `save_loaded(slot)` when the state leaves 0xd / 0xe (confirmed on a real Load: 0xd, 0xe with the file open, then the level load).
//! The slot is the `saveN.bin` of an open card file (`Mc::fds`), else the target slot last seen queued with the command (manual save), else the
//! current slot (autosave, proven on a real ship-travel autosave: state 0x2 cmd 0xf, 0xf, then 0x10 with save1.bin open, current slot 1). A manual save whose slot shows nowhere in the entering field waits for the card file to be opened (next field). Level 'Retail::load' machines start the words from 0; a jump is then just two ordinary transitions.
//!
//! RAC_OMNI_TRACE prints every change of these words (`omni-trace: tick N card ...`) so the states can be checked on a real run.

use ee_oracle::cpu::Cpu;

const STATE: usize = 0x13_d364;
const CMD: usize = 0x13_d36c;
const TARGET: usize = 0x13_d370;
const CURRENT: usize = 0x13_d2a4;

/// Writing states: autosave and whole-image write.
pub fn is_writing(state: u32) -> bool {
	matches!(state, 0x10 | 0x13 | 0x14)
}

/// Full-load states.
pub fn is_loading(state: u32) -> bool {
	matches!(state, 0xd | 0xe)
}

/// The words one field shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Words {
	pub state: u32,
	pub cmd: i32,
	pub target: i32,
	/// The game's current save slot (word 0x13d2a4), -1 or out of 0..=4 when none.
	pub cur: i32,
	/// The slot of the `saveN.bin` card file the game has open, if any.
	pub fd_slot: Option<i32>,
	/// The read / write position of that file (trace only: shows no byte was written when the slot is first seen).
	pub fd_pos: u32,
}

/// What the card did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardEvent {
	Saving(i32),
	SaveLoaded(i32),
}

/// `"save3.bin"` -> 3; anything else None. Slots are 0..=4 in the game.
pub fn slot_of_name(name: &str) -> Option<i32> {
	let n = name.strip_prefix("save")?.strip_suffix(".bin")?;
	if n.len() != 1 {
		return None;
	}
	n.parse().ok()
}

#[derive(Debug, Default)]
pub struct CardWatcher {
	/// The state of the last field (None before the first).
	prev: Option<u32>,
	/// The target slot last seen queued, and the command it came with.
	queued: Option<(i32, i32)>,
	/// The slot of the load in progress (an open file or the current slot, remembered while the load runs).
	loading: Option<i32>,
	/// A manual save was entered and its slot is not known yet: the next field with the card file open reports it.
	pending: bool,
	/// The words of the last field (for the trace line).
	last: Option<Words>,
}

impl CardWatcher {
	/// True when the words differ from the last field's (the trace prints a line then).
	pub fn changed(&mut self, w: &Words) -> bool {
		self.last.replace(*w) != Some(*w)
	}

	/// The events of this field.
	pub fn step(&mut self, w: &Words) -> Vec<CardEvent> {
		let mut out = Vec::new();
		if w.target >= 0 && w.cmd >= 0 {
			self.queued = Some((w.target, w.cmd));
		}
		let prev = self.prev.replace(w.state).unwrap_or(0);
		let cur = Some(w.cur).filter(|a| (0..5).contains(a));
		if is_writing(w.state) && !is_writing(prev) {
			let manual = self.queued.filter(|&(_, cmd)| cmd == 0x13).map(|(t, _)| t);
			self.queued = None;
			if w.state == 0x10 {
				// An autosave goes to the slot its file is open on, else the current slot (never the queued target: for command 0xf it is the card port).
				out.extend(w.fd_slot.or(cur).or(manual).map(CardEvent::Saving));
			} else if let Some(s) = w.fd_slot.or(manual) {
				out.push(CardEvent::Saving(s));
			} else {
				// A whole-image (manual) save: the state machine took the queued target in the very field the menu queued it (the words
				// never show it) and the write goes to the slot's existing file, which is open from the next field on (real run: state 0x13,
				// then 0x14 with save4.bin open while the current-slot word was not the file's). Wait for the file; its slot is the slot saved.
				self.pending = true;
			}
		}
		if self.pending {
			if let Some(s) = w.fd_slot.filter(|_| is_writing(w.state)) {
				out.push(CardEvent::Saving(s));
				self.pending = false;
			} else if !is_writing(w.state) {
				// The write ended without a card file ever seen open: the current slot is the best answer left.
				self.pending = false;
				out.extend(cur.map(CardEvent::Saving));
			}
		}
		if is_loading(w.state) {
			if let Some(s) = w.fd_slot.or(self.queued.map(|(t, _)| t)).or(Some(w.cur).filter(|a| (0..5).contains(a))) {
				self.loading = Some(w.fd_slot.unwrap_or(s));
			}
		} else if is_loading(prev) {
			if let Some(s) = self.loading.take() {
				out.push(CardEvent::SaveLoaded(s));
			}
		}
		out
	}
}

fn rd(ram: &[u8], a: usize) -> u32 {
	ram.get(a..a + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// The words out of the machine: RAM and the open card files.
pub fn read(c: &Cpu) -> Words {
	let fd = c.mc.fds.iter().flatten().find_map(|f| f.path.last().and_then(|n| slot_of_name(n)).map(|s| (s, f.pos)));
	let (fd_slot, fd_pos) = (fd.map(|f| f.0), fd.map_or(0, |f| f.1));
	Words { state: rd(&c.mem.ram, STATE), cmd: rd(&c.mem.ram, CMD) as i32, target: rd(&c.mem.ram, TARGET) as i32, cur: rd(&c.mem.ram, CURRENT) as i32, fd_slot, fd_pos }
}

#[cfg(test)]
mod tests {
	use super::*;

	fn w(state: u32, cmd: i32, target: i32, cur: i32, fd: Option<i32>) -> Words {
		Words { state, cmd, target, cur, fd_slot: fd, fd_pos: 0 }
	}

	#[test]
	fn slot_names() {
		assert_eq!(slot_of_name("save0.bin"), Some(0));
		assert_eq!(slot_of_name("save4.bin"), Some(4));
		assert_eq!(slot_of_name("save12.bin"), None);
		assert_eq!(slot_of_name("icon.sys"), None);
		assert_eq!(slot_of_name("save.bin"), None);
	}

	#[test]
	fn an_autosave_reports_saving_with_the_current_slot_when_the_write_state_is_entered() {
		let mut c = CardWatcher::default();
		assert!(c.step(&w(1, -1, -1, 2, None)).is_empty());
		// The state machine took the command 0xf in this field: the next states show the write.
		assert_eq!(c.step(&w(0x10, -1, -1, 2, None)), vec![CardEvent::Saving(2)]);
		// Still writing in the next fields: nothing more.
		assert!(c.step(&w(0x10, -1, -1, 2, Some(2))).is_empty());
		assert!(c.step(&w(1, -1, -1, 2, None)).is_empty());
		// Again later: a second event.
		assert_eq!(c.step(&w(0x10, -1, -1, 2, None)), vec![CardEvent::Saving(2)]);
	}

	#[test]
	fn a_manual_save_uses_the_queued_target_or_the_open_file() {
		let mut c = CardWatcher::default();
		c.step(&w(1, 0x13, 3, 0, None));
		assert_eq!(c.step(&w(0x13, -1, -1, 0, None)), vec![CardEvent::Saving(3)]);
		assert!(c.step(&w(0x14, -1, -1, 0, Some(3))).is_empty());
		// With the file open the open file wins.
		let mut c = CardWatcher::default();
		assert_eq!(c.step(&w(0x13, -1, -1, 0, Some(4))), vec![CardEvent::Saving(4)]);
		// No slot known at all: nothing is sent for an autosave.
		let mut c = CardWatcher::default();
		assert!(c.step(&w(0x10, -1, -1, -1, None)).is_empty());
	}

	#[test]
	fn a_manual_save_whose_target_was_never_seen_waits_for_the_open_file_not_the_current_slot() {
		// The real run: the menu queued the command and the state machine took it in the same field, state 0x13 with the current-slot word 0,
		// then 0x14 with save4.bin open for the write.
		let mut c = CardWatcher::default();
		assert!(c.step(&w(2, -1, -1, 0, None)).is_empty());
		assert!(c.step(&w(0x13, -1, -1, 0, None)).is_empty());
		assert_eq!(c.step(&w(0x14, -1, -1, 0, Some(4))), vec![CardEvent::Saving(4)]);
		assert!(c.step(&w(0x14, -1, -1, 0, Some(4))).is_empty());
		assert!(c.step(&w(2, -1, -1, 0, None)).is_empty());
		// The write ended and no file was ever seen: the current slot.
		let mut c = CardWatcher::default();
		assert!(c.step(&w(0x13, -1, -1, 1, None)).is_empty());
		assert_eq!(c.step(&w(2, -1, -1, 1, None)), vec![CardEvent::Saving(1)]);
	}

	#[test]
	fn the_real_ship_travel_autosave_reports_saving_once_with_the_open_file_slot_before_any_byte() {
		// Real run (cov/story_ship_travel_1063, Eudora -> Novalis): state 2 with cmd 0xf queued (target 0 = the card port, not the slot),
		// then state 0xf, then state 0x10 with save1.bin open at pos 0 while the current-slot word said 1.
		let mut c = CardWatcher::default();
		assert!(c.step(&w(2, -1, -1, 1, None)).is_empty());
		assert!(c.step(&w(2, 0xf, 0, 1, None)).is_empty());
		assert!(c.step(&w(0xf, -1, -1, 1, None)).is_empty());
		assert_eq!(c.step(&w(0x10, -1, -1, 1, Some(1))), vec![CardEvent::Saving(1)]);
		for pos in [8, 5432, 16328, 19052] {
			let mut x = w(0x10, -1, -1, 1, Some(1));
			x.fd_pos = pos;
			assert!(c.step(&x).is_empty());
		}
		assert!(c.step(&w(0, -1, -1, 1, None)).is_empty());
		// File not open yet when 0x10 is entered: the current slot, not the queued port target.
		let mut c = CardWatcher::default();
		c.step(&w(2, 0xf, 0, 3, None));
		assert_eq!(c.step(&w(0x10, -1, -1, 3, None)), vec![CardEvent::Saving(3)]);
	}

	#[test]
	fn a_load_reports_save_loaded_when_the_load_states_are_left_not_for_the_summary_scan() {
		let mut c = CardWatcher::default();
		assert!(c.step(&w(0x17, -1, -1, -1, Some(1))).is_empty());
		assert!(c.step(&w(1, -1, -1, -1, None)).is_empty());
		assert!(c.step(&w(0xd, -1, -1, -1, Some(1))).is_empty());
		assert!(c.step(&w(0xe, -1, -1, -1, Some(1))).is_empty());
		assert_eq!(c.step(&w(1, -1, -1, 1, None)), vec![CardEvent::SaveLoaded(1)]);
		assert!(c.step(&w(1, -1, -1, 1, None)).is_empty());
	}

	#[test]
	fn saving_comes_before_the_card_file_is_written_that_is_in_the_field_the_write_state_is_seen() {
		// The event is produced from the state word alone: the file (fd) is not open yet when the state is first seen.
		let mut c = CardWatcher::default();
		let first = c.step(&w(0x10, -1, -1, 4, None));
		assert_eq!(first, vec![CardEvent::Saving(4)]);
	}

	#[test]
	fn the_machine_words_are_read_from_ram_and_the_open_card_files() {
		let mut c = Cpu::new(ee_oracle::mem::Mem::new());
		c.mem.ram[STATE..STATE + 4].copy_from_slice(&0x10u32.to_le_bytes());
		c.mem.ram[CMD..CMD + 4].copy_from_slice(&(-1i32).to_le_bytes());
		c.mem.ram[CURRENT..CURRENT + 4].copy_from_slice(&2u32.to_le_bytes());
		c.mc.fds = vec![None, Some(ee_oracle::mc::Fd { port: 0, path: vec!["BESCES-50916RATCHET".into(), "save2.bin".into()], pos: 0 })];
		let words = read(&c);
		assert_eq!((words.state, words.cmd, words.cur, words.fd_slot), (0x10, -1, 2, Some(2)));
	}
}
