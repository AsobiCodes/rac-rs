//! [PC] Quit Game on the title's main menu: a fourth row under New Game, Load Game and Options that closes the program.
//!
//! The main menu is the front end's root page (PAL 0x1d4548 in the boot ELF, also in frontbin): 14 slot sequences (u32 each,
//! slots 2..4 play the class-0x472 button seqs 0x10..0x12, slot 5 parks on seq 5), id 0x2e at +0x3c, the current item at
//! +0x40 and the 14 slot items from +0x44. Each button is a 0x50-byte item whose +0x34 points at one 24-byte record
//! `{text id | kind << 16, target}` (kind 3 opens page `target`, as Options does) and whose +0x38 / +0x3c are its Up / Down
//! neighbours. Seq 0x13 is the next button down the same column (the pause menu's fifth row, y 316), so slot 5 takes it and
//! a copy of the Options button: its record opens `QPAGE`, a copy of the root page that only marks the choice. The worker
//! sees the front end's page pointer reach it (`front_field`) and the window closes (viewer/handler.rs), as its close
//! button would. The label is retail's own "Quit Game" (0x4f26, the pause menu's row), so it is in every language.
//!
//! Reserved RAM: `RANGE`, inside pcext's BASE window, above pcdisp's Display page pool (BASE+0xE000..0xE400) (the menu reader accepts it; the canary of prompts.rs skips it).
use super::pcext::BASE;
use ee_oracle::mem::Mem;
use std::sync::atomic::{AtomicBool, Ordering};

pub const RANGE: std::ops::Range<u32> = BASE + 0xF000..BASE + 0xF200;
/// The new button item, its record (and the zero record after it) and the page it opens.
const QITEM: u32 = BASE + 0xF000;
const QRECS: u32 = BASE + 0xF060;
const QPAGE: u32 = BASE + 0xF100;
const ROOT_ID: u32 = 0x2e;
/// The three retail records: New Game (kind 4), Load Game (kind 5), Options (kind 3).
const RECS: [u32; 3] = [0x0004_4f2a, 0x0005_4f28, 0x0003_4ee7];
pub const QUIT_GAME: u32 = 0x4f26;
const SEQ_PARK: u32 = 5;
const SEQ_ROW4: u32 = 0x13;

/// The row is on the title's main menu (the mirror lists it; frontend.rs).
static ROW: AtomicBool = AtomicBool::new(false);
/// The player chose it.
static QUIT: AtomicBool = AtomicBool::new(false);

pub fn row_shown() -> bool {
	ROW.load(Ordering::Relaxed)
}

/// True once the player picked Quit Game: the window closes.
pub fn requested() -> bool {
	QUIT.load(Ordering::Relaxed)
}

/// The root page in RAM: the page whose slots 2..4 play seqs 0x10..0x12 and hold the three retail buttons. Searched rather than
/// taken from a fixed address: the boot ELF's copy and frontbin's sit at different addresses on each disc.
fn find_root(mem: &mut Mem) -> Option<u32> {
	let ok = |a: u32| (0x10_0000..0x200_0000).contains(&a);
	// Both copies lie in the code and data below 4 MB (the same range pcext's `hook_front` scans).
	(0x10_0000..0x40_0000 - 0x80).step_by(4).find(|&a| {
		if mem.rd32(a + 0x3c) != ROOT_ID || mem.rd32(a + 8) != 0x10 || mem.rd32(a + 0xc) != 0x11 || mem.rd32(a + 0x10) != 0x12 {
			return false;
		}
		(0..3).all(|k| {
			let it = mem.rd32(a + 0x4c + 4 * k);
			ok(it) && { let r = mem.rd32(it + 0x34); ok(r) && mem.rd32(r) == RECS[k as usize] }
		})
	})
}

/// Called when the front end starts (retail.rs, next to pcext's `hook_front`): adds the row to the root page. Already there
/// (the boot ELF's page after a level, which keeps it) is fine; a page that does not look like retail's is left alone.
pub fn hook_front(mem: &mut Mem) {
	QUIT.store(false, Ordering::Relaxed);
	let Some(root) = find_root(mem) else {
		ROW.store(false, Ordering::Relaxed);
		eprintln!("retail: title Quit Game: root page not found, row not added");
		return;
	};
	let slot5 = mem.rd32(root + 0x58);
	if slot5 != 0 && slot5 != QITEM || ![SEQ_PARK, SEQ_ROW4].contains(&mem.rd32(root + 0x14)) {
		ROW.store(false, Ordering::Relaxed);
		eprintln!("retail: title Quit Game: root page {root:#x} slot 5 in use, row not added");
		return;
	}
	let (new_game, options) = (mem.rd32(root + 0x4c), mem.rd32(root + 0x54));
	// The button: Options' item with our record, between Options (Up) and New Game (Down), so the column still wraps.
	for o in (0..0x50).step_by(4) {
		let w = mem.rd32(options + o);
		mem.wr32(QITEM + o, w);
	}
	mem.wr32(QITEM + 0x34, QRECS);
	mem.wr32(QITEM + 0x38, options);
	mem.wr32(QITEM + 0x3c, new_game);
	for (i, w) in [QUIT_GAME | 3 << 16, QPAGE, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0].into_iter().enumerate() {
		mem.wr32(QRECS + 4 * i as u32, w);
	}
	// The page it opens: the root page as it is, so a field that draws it before the window closes draws the main menu.
	for o in (0..0x80).step_by(4) {
		let w = mem.rd32(root + o);
		mem.wr32(QPAGE + o, w);
	}
	mem.wr32(options + 0x3c, QITEM);
	mem.wr32(new_game + 0x38, QITEM);
	mem.wr32(root + 0x14, SEQ_ROW4);
	mem.wr32(root + 0x58, QITEM);
	ROW.store(true, Ordering::Relaxed);
	eprintln!("retail: title Quit Game row added to root page {root:#x}");
}

/// Worker, before each field of the front end: the page pointer on `QPAGE` means Cross on Quit Game.
pub fn front_field(mem: &mut Mem) {
	if row_shown() && !requested() && mem.rd32(super::disc::va(super::mouse_menu::FRONT_PAGE_PTR)) == QPAGE {
		QUIT.store(true, Ordering::Relaxed);
		eprintln!("retail: title Quit Game chosen, closing");
	}
}
