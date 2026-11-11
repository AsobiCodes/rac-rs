//! The mouse's own invert rows (v0.5.1, GitHub PR #48): "Mouse up/down" and "Mouse left/right", each Like camera (the default) /
//! Normal / Inverted. Retail's Reversed camera options (Options -> Camera Options, Left/Right and Up/Down) flip the right stick AND
//! the mouse in the Modern camera (`modcam_turn.rs`); these rows let the mouse deviate from them. Like camera = exactly that
//! behaviour (nothing is written to the options file, the turn maths gets the very same signs); Normal = the mouse turns as retail's
//! Normal does (right = look right, up = look up); Inverted = as Reversed does. Only the mouse is affected: the stick keeps following
//! retail's options. Used by the Modern camera (`modcam.rs`, `modcam/fp_look.rs`, `modcam/late_look.rs`) and the ship mouse's vertical
//! axis (`shipmouse.rs`).
//!
//! The classic Controls page's PC section holds the two rows (bytes `UD_B` / `LR_B` in pcctl's block: 0 Like camera, 1 Normal,
//! 2 Inverted, so a default RAM stays zero; `sync` hands a change to the options file), the modern Controls table holds them too
//! (`ui_bridge` ids `mouse_ud` / `mouse_lr`). File keys `mouse_ud=normal|inverted`, `mouse_lr=normal|inverted` (extra keys of
//! pc_options.txt: Like camera removes the key, older files and older builds are unaffected).
use super::pcctl::CBASE;
use super::pcopts::PcOptions;
use ee_oracle::mem::Mem;
use std::sync::atomic::{AtomicU8, Ordering};

/// The rows' bytes (pcctl's block): up/down, left/right.
pub(super) const UD_B: u32 = CBASE + 0x22;
pub(super) const LR_B: u32 = CBASE + 0x23;
/// The value texts' ids (Like camera, Normal, Inverted): 0x7f47..0x7f49, the unused label ids of pcctl's pool (a page has at most `ROWS` = 7
/// rows, labels 0x7f40..0x7f46). Not 0x7fab..0x7fae: those are the Button glyphs row's (glyphs.rs).
pub(super) const ID_V0: u32 = 0x7f47;
pub(super) const ID_V_END: u32 = 0x7f4a;

pub const LIKE: u8 = 0;
pub const NORMAL: u8 = 1;
pub const INVERTED: u8 = 2;

/// The options-file keys of axis 0 (up/down) and 1 (left/right).
const KEYS: [&str; 2] = ["mouse_ud", "mouse_lr"];
static MODE: [AtomicU8; 2] = [AtomicU8::new(LIKE), AtomicU8::new(LIKE)];

type T5 = [&'static [u8]; 5];
/// The labels (axis 0, 1) in the game's font bytes (Es o-acute 0x9c, It u-grave 0xa5).
#[rustfmt::skip]
static LABEL: [T5; 2] = [
	[b"Mouse up/down", b"Souris haut/bas", b"Maus hoch/runter", b"Rat\x9cn arriba/abajo", b"Mouse su/gi\xa5"],
	[b"Mouse left/right", b"Souris gauche/droite", b"Maus links/rechts", b"Rat\x9cn izq./der.", b"Mouse sin./des."],
];
/// The values (Like camera, Normal, Inverted); Fr e-acute 0x8c, Es a-acute 0x84.
#[rustfmt::skip]
static VALUE: [T5; 3] = [
	[b"Like camera", b"Comme cam\x8cra", b"Wie Kamera", b"Como c\x84mara", b"Come camera"],
	[b"Normal", b"Normal", b"Normal", b"Normal", b"Normale"],
	[b"Inverted", b"Invers\x8c", b"Invertiert", b"Invertido", b"Invertito"],
];

/// `pcext::init`: the file's values are the start values (anything unknown = Like camera).
pub fn init(opts: &PcOptions) {
	for (a, key) in KEYS.iter().enumerate() {
		let m = match opts.extra_get(key).map(|v| v.trim().to_ascii_lowercase()).as_deref() {
			Some("normal") => NORMAL,
			Some("inverted") => INVERTED,
			_ => LIKE,
		};
		MODE[a].store(m, Ordering::Relaxed);
	}
}

/// The mode of axis `a` (0 up/down, 1 left/right).
pub fn mode(a: usize) -> u8 {
	MODE[a.min(1)].load(Ordering::Relaxed).min(INVERTED)
}

/// The mouse's turn sign of one axis for the mode `m` and retail's sign `retail` (-1.0 Reversed, 1.0 Normal).
pub fn sign_for(m: u8, retail: f32) -> f32 {
	match m {
		NORMAL => 1.0,
		INVERTED => -1.0,
		_ => retail,
	}
}

/// The mouse's (left/right, up/down) signs from retail's own (-1.0 Reversed, 1.0 Normal), for the Modern camera's turn.
pub fn signs(lr: f32, ud: f32) -> (f32, f32) {
	(sign_for(mode(1), lr), sign_for(mode(0), ud))
}

/// The ship mouse's vertical axis: true = inverted, given the game's own Y option (`inverted` = Reversed).
pub fn ud_inverted(retail_inverted: bool) -> bool {
	sign_for(mode(0), if retail_inverted { -1.0 } else { 1.0 }) < 0.0
}

/// The text of `id` in language `l`: a value text (`ID_V0..ID_V_END`); empty for any other id.
pub fn value_text(id: u32, l: usize) -> Vec<u8> {
	if (ID_V0..ID_V_END).contains(&id) { VALUE[(id - ID_V0) as usize][l.min(4)].to_vec() } else { Vec::new() }
}

/// The label of axis `a` in language `l`.
pub fn label(a: usize, l: usize) -> Vec<u8> {
	LABEL[a.min(1)][l.min(4)].to_vec()
}

/// The classic page was just built on fresh RAM: its bytes show the running modes.
pub(super) fn seed(mem: &mut Mem) {
	mem.wr8(UD_B, mode(0));
	mem.wr8(LR_B, mode(1));
}

fn store(opts: &mut PcOptions, a: usize, m: u8) -> bool {
	if MODE[a].swap(m, Ordering::Relaxed) == m {
		return false;
	}
	eprintln!("retail: PC options: {} {}", ["Mouse up/down", "Mouse left/right"][a], ["Like camera", "Normal", "Inverted"][m as usize]);
	opts.extra_set(KEYS[a], (m != LIKE).then(|| if m == NORMAL { "normal" } else { "inverted" }.to_string()));
	true
}

/// Worker, every field after the page patch: a changed row byte goes into the options file (true = the caller saves).
pub fn sync(mem: &mut Mem, opts: &mut PcOptions) -> bool {
	if !super::pcctl::controls_built(mem) {
		return false;
	}
	let mut save = false;
	for (a, at) in [UD_B, LR_B].into_iter().enumerate() {
		save |= store(opts, a, mem.rd8(at).min(INVERTED));
	}
	save
}

/// The modern Controls table's row (`ui_bridge` `mouse_ud` / `mouse_lr`): the page's byte follows when it exists; true = the caller saves.
pub fn set_from_ui(mem: &mut Mem, opts: &mut PcOptions, a: usize, m: u8) -> bool {
	let (a, m) = (a.min(1), m.min(INVERTED));
	if super::pcctl::controls_built(mem) {
		mem.wr8([UD_B, LR_B][a], m);
	}
	store(opts, a, m)
}

#[cfg(test)]
#[path = "pcmouseinv_tests.rs"]
mod tests;
