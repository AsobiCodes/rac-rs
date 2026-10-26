//! The front end (frontbin) as the mirror sees it: its RAM state and modal dialog, the load-loop test, `front_snap`,
//! and the GS-full pipe the front end switches on late.

use super::*;

/// The front end's GS-full pipe (title_boot.rs): the jobs of the machine go to it from now on (the mirror's HUD capture only
/// keeps getting the field-end markers). false: the pipe could not start (GS.bin and the VU1 files missing in `dir`).
pub(super) fn start_late_gs(m: &mut ee_oracle::Machine, dir: &str, gtx: &mut Option<std::sync::mpsc::SyncSender<crate::gsfull::pipe::FieldIn>>, jobs_rx: &mut Option<std::sync::mpsc::Receiver<ee_oracle::gs::Job>>) -> bool {
	let Some((tx, shared)) = crate::gsfull::pipe::start(dir) else {
		eprintln!("retail: front end: the GS-full pipe cannot start (no GS.bin / VU1 files in {dir}); no picture of the front end");
		return false;
	};
	let (jtx, jrx) = std::sync::mpsc::channel::<ee_oracle::gs::Job>();
	m.cpu.mem.gsq = Some(jtx);
	(*gtx, *jobs_rx) = (Some(tx), Some(jrx));
	title_boot::set_late(Some(shared));
	eprintln!("retail: front end: GS-full pipe started for the game's own picture");
	true
}

/// The front end's state the mirror draws its picture from (title scene tick, logo and Press Start fades, save slots; all
/// read from frontbin's RAM, `front_snap`). Addresses: frontend_flow.md section 2, frontbin.elf.c FUN_001eafa8 / FUN_001eb090.
#[derive(Clone, Debug, Default)]
pub struct FrontRam {
	/// Title timeline tick DAT_0018cad4 (50 Hz, wraps at DAT_0018cae0).
	pub tick: u32,
	/// Logo alpha DAT_0015ef50 and Press Start alpha DAT_0015ef54 (PS2 alpha, 0x80 = opaque).
	pub logo: u32,
	pub press: u32,
	/// The black fade float DAT_0015f43c (0 none, 1 = fully black; FUN_001f4e50 draws a black quad of alpha 128 * it).
	pub fade: f32,
	/// Language word DAT_0015ed88.
	pub lang: u32,
	/// The five slot summaries at 0x13d2b0 (stride 0x1c): level (-1 empty), bolts, playtime frames, date bytes (+0x15..0x17).
	pub slots: Vec<Option<(u32, u32, u32, [u8; 3])>>,
	/// Cursor words (+0x40) of the slot-list items of the Load Game page (0x1d17b0) and the New Game page (0x1d4ce8) [I: same layout as the other list items].
	pub slot_cur: [u32; 2],
	/// The modal message box (mode 4) words.
	pub dialog: FrontDialog,
}

/// The front end's modal message box (mode 4; FUN_001fb5e8 opens it, FUN_001fc8f0 runs it, FUN_001fb780 draws it): the words it
/// draws from. `kind` DAT_00193280 (3 = memory card messages), `state` the card state DAT_0015eeb0, `flags` DAT_0015eeb4, `card`
/// DAT_0015f5c8, fade-in counter DAT_00193284, box / hint timers DAT_001932a0 / DAT_001932a4, 60 Hz step DAT_0019329c, screen height DAT_0013e504.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrontDialog {
	/// The front end's mode word DAT_0015f5e4: 4 = the box is up.
	pub mode: u32,
	pub kind: u32,
	pub state: u32,
	pub flags: u32,
	pub card: u32,
	pub fade_in: u32,
	pub box_t: u32,
	pub hint_t: u32,
	pub step: u32,
	pub height: u32,
}

/// The snapshot of a field in the front end (frontbin): no level to mirror, the front end's mode word and menu.
/// Return addresses behind the three `jal FUN_00122298` (the vblank wait) of the level-load loops of FUN_00231b00 (boot ELF; the
/// loops at 0x2321b0 / 0x2321f8 / 0x232238, decomp/notes/loading.md): while one of them is on the game thread's stack, the front
/// end only waits for the level data to arrive, it reads no pad and shows no change that depends on the wall clock [C].
const LOAD_LOOP_RA: [u32; 3] = [0x2321b8, 0x232208, 0x232248];

/// `LOAD_LOOP_RA` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const LOAD_LOOP_RA_USA: [u32; 3] = [0x2326a8, 0x2326f8, 0x232738];

/// `LOAD_LOOP_RA` of frontbin's copy of the loops (FUN_002314b8, the same code 0x648 lower): the front end after a Quit Game runs
/// from frontbin, the boot ELF's copy only carries the cold-boot title [C].
const LOAD_LOOP_RA_FRONTBIN: [u32; 3] = [0x231b70, 0x231bc0, 0x231c00];

/// `LOAD_LOOP_RA_FRONTBIN` on the USA disc (disc_map `FRONT`: one aligned run there, USA = PAL + 0x4f0).
const LOAD_LOOP_RA_FRONTBIN_USA: [u32; 3] = [0x232060, 0x2320b0, 0x2320f0];

/// The front end's game thread is parked inside one of the load loops (see `LOAD_LOOP_RA`): its stack above `sp` (and `ra`) holds a
/// return address into FUN_00231b00's wait calls (`frontbin`: into FUN_002314b8's, the front end after a Quit Game).
pub(super) fn in_load_loop(m: &ee_oracle::Machine, frontbin: bool) -> bool {
	if m.rd32(title_boot::front_mode()) != 6 {
		return false;
	}
	let ras = if frontbin { crate::retail::disc::pick(&LOAD_LOOP_RA_FRONTBIN, &LOAD_LOOP_RA_FRONTBIN_USA) } else { crate::retail::disc::pick(&LOAD_LOOP_RA, &LOAD_LOOP_RA_USA) };
	let ra = m.cpu.g(31) as u32;
	if ras.contains(&ra) {
		return true;
	}
	let sp = m.cpu.g(29) as u32;
	(0..0x100u32).any(|k| {
		let a = sp.wrapping_add(k * 8);
		a < 0x200_0000 - 8 && m.rd32(a + 4) == 0 && ras.contains(&m.rd32(a))
	})
}

pub(super) fn front_snap(m: &ee_oracle::Machine) -> Snap {
	let mode = m.rd32(title_boot::front_mode());
	let slots = (0..5u32)
		.map(|i| {
			let a = 0x13d2b0 + i * 0x1c;
			let level = m.rd32(a);
			(level < 19).then(|| (level, m.rd32(a + 4), m.rd32(a + 0xc), [m.rd8(a + 0x15), m.rd8(a + 0x16), m.rd8(a + 0x17)]))
		})
		.collect();
	let front_ram = FrontRam { tick: m.rd32(va(0x18cad4)), logo: m.rd32(0x15ef50), press: m.rd32(0x15ef54), fade: f32::from_bits(m.rd32(0x15f43c)), lang: m.rd32(0x15ed88), slots, slot_cur: [m.rd32(va(0x1d17b0) + 0x40), m.rd32(va(0x1d4ce8) + 0x40)], dialog: FrontDialog { mode, kind: m.rd32(va(0x193280)), state: m.rd32(0x15eeb0), flags: m.rd32(0x15eeb4), card: m.rd32(va(0x15f5c8)), fade_in: m.rd32(va(0x193284)), box_t: m.rd32(va(0x1932a0)), hint_t: m.rd32(va(0x1932a4)), step: m.rd32(va(0x19329c)), height: m.rd32(0x13e504) } };
	let menu = title_boot::front_menu(m, mode);
	prompts::set_menu(menu.is_some());
	menu_keys::publish(menu.as_ref().map(menu_keys::screen));
	// The front end's message box (mode 4: memory card, 60 Hz) is no page menu, but its keys and prompts work the menu way
	// (menu_keys::front_dialog: Enter / Space / left click = Yes / Continue, Esc = No), like the in-level mode-4 dialogs.
	let dialog = if menu.is_none() && mode == 4 { crate::frontend::dialog_keys(&front_ram.dialog) } else { None };
	if dialog.is_some() {
		prompts::set_menu(true);
		menu_keys::publish(dialog);
	}
	Snap { front: true, hold: true, mode, level: u32::MAX, menu, dialog, front_ram: Some(front_ram), pad: [m.rd32(0x13cae0), m.rd32(0x13cae4), 0, 0], ..Snap::default() }
}
