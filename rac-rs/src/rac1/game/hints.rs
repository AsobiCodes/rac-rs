//! The Novalis hint director (class 1341, `HelpHintDirectorUpdate` OV:165853-166145) and the
//! per-slot help records it reads (decomp/notes/novalis_flow.md "Hint director (class 1341)").
//!
//! Save record 16 = 148 help slots, record 17 = 36 move counters; both are 8-byte entries
//! {s16 count (-1 = disabled), u16 playtime/600, u32 level mask | 0x80000000}. Counter n lives at
//! index n (n9 map opened, n11 hard Cross, n12 drowning, n13 nanotech tip, n22/n23 swim areas,
//! n29 Cross mash). Times are 60 Hz frames, the unit the game counts playtime in.

pub const SLOTS: usize = 148;
pub const COUNTERS: usize = 36;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Entry {
	pub count: i16,
	pub minutes: u16,
	pub mask: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HelpRecords {
	pub slots: Vec<Entry>,
	pub counters: Vec<Entry>,
}

impl Default for HelpRecords {
	fn default() -> Self {
		HelpRecords { slots: vec![Entry::default(); SLOTS], counters: vec![Entry::default(); COUNTERS] }
	}
}





impl HelpRecords {
	/// A death re-arms the one-shot help messages: bit 31 of every slot's level mask is cleared
	/// (FUN_00225400, called by the die function FUN_00231478).
	pub fn rearm(&mut self) {
		for e in &mut self.slots {
			e.mask &= 0x7fff_ffff;
		}
	}









	/// Slots shown at least once (the save-anywhere `seen` list).
	pub fn seen(&self) -> Vec<i16> {
		self.slots.iter().enumerate().filter(|(_, e)| e.count > 0).map(|(i, _)| i as i16).collect()
	}
}






