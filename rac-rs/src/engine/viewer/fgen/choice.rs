//! [PC] The player's frame-generation choice (lane fgen, stage S4; no retail counterpart): the Frame Generation row of the game's
//! menus (the game pack's menu rows: a modern Graphics row and a classic Graphics row).
//! Kept in the options file only (`framegen=off|fsr`): no RAM byte of its own, so the game and its RAM hash never see it. Read once
//! before the window's device is made (`mod.rs` `mode`: the env RAC_FG overrides it, for tests): that value's maker (FSR or DLSS)
//! makes the run's swapchain. Lane fglive (follow.rs): the row offers only the running Upscaler's maker, the value in force
//! follows the Upscaler, and changes within the running maker (Off, DLSS 2x/3x/4x) apply at once; another maker after a restart
//! (modern: the row's note; classic: "- restart").
//! A value this PC cannot run (its DLLs are not next to the exe, or the runtime refused at start) is not offered (ui/live.rs
//! `offer`); with nothing but Off left the classic row reads "Not available". The file keeps the choice.
//! S3 (DLSS frame generation) appends its values to `KINDS` (`dlss2`, `dlss3`, `dlss4`; at most 4x, user 2026-11-06) and withdraws
//! the ones the card cannot run (RTX 40: 2x; RTX 50: 2x..4x) with `set_runs`.

use crate::pcopts::PcOptions;
use std::sync::atomic::{AtomicU8, Ordering};

/// Options file key of the Frame Generation row.
pub const KEY: &str = "framegen";
/// The row's values in the file, in row order: Off, `fsr` (AMD FidelityFX frame generation, one generated frame per rendered
/// one = 2x). S3 appends the DLSS multipliers here (and their names in the menus' value lists).
pub const KINDS: [&str; 5] = ["off", "fsr", "dlss2", "dlss3", "dlss4"];
/// The row value of `fsr`.
pub const FSR_VALUE: i32 = 1;
/// [S3] The row values of NVIDIA DLSS frame generation 2x, 3x, 4x (`dlss2..4`; generated frames per rendered one = value - 1).
pub const DLSS_VALUES: [i32; 3] = [2, 3, 4];
/// The FidelityFX frame-generation DLLs, either backend (ffx_fg.rs loads the pair of the running one).
#[cfg(windows)]
const FSR_DLLS: [[&str; 2]; 2] = [
	["amd_fidelityfx_loader_vk.dll", "amd_fidelityfx_framegeneration_vk.dll"],
	["amd_fidelityfx_loader_dx12.dll", "amd_fidelityfx_framegeneration_dx12.dll"],
];

/// The chosen row value (the options file's, or the menu's since).
static CHOSEN: AtomicU8 = AtomicU8::new(0);
/// The value this run started with (the file's at `init`): a chosen value that differs from it applies after a restart.
static STARTED: AtomicU8 = AtomicU8::new(0);
/// Bit v set: row value v cannot run on this PC (`set_runs`).
static NOT_RUNNING: AtomicU8 = AtomicU8::new(0);
/// Bit v set: the engine refused value v during this run (`refuse`): `init` never offers it again.
static REFUSED: AtomicU8 = AtomicU8::new(0);
/// [S3] The most generated frames this PC's card runs (0: no DLSS frame generation; 3 = 4x, the default until a start found out):
/// remembered across runs in the player dir's `CARD_FILE` (`remember_card`), so a card without DLSS-G (RTX 30) stops offering the
/// DLSS values after the first start that tried one, and nothing probes the card while frame generation is Off (Off = today).
static CARD_MAX: AtomicU8 = AtomicU8::new(3);
/// [S3] The card memory: `<adapter name>\t<max generated frames>\t<driver version>` (`format_card`); another card or another
/// driver (`card_seen`) forgets it, so a refusal a driver update may lift is asked again.
const CARD_FILE: &str = "framegen_card.txt";

/// [S3] The remembered card: what Streamline found it runs, with the driver it found it on.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CardMemory {
	pub name: String,
	pub max: u8,
	pub driver: String,
}

impl CardMemory {
	/// Whether this memory is about adapter `name` on driver `driver` (pure).
	pub(crate) fn is(&self, name: &str, driver: &str) -> bool {
		self.name == name && self.driver == driver
	}
}
/// [PC] lane diag: the row's note while frame-generation files are missing beside the exe (a player copied only the exe), in place
/// of "Applies after a restart".
pub const MISSING_NOTE: crate::ui::T5 = [
	"Files missing next to the game: copy the whole game folder",
	"Fichiers manquants \u{e0} c\u{f4}t\u{e9} du jeu : copiez tout le dossier du jeu",
	"Dateien neben dem Spiel fehlen: den ganzen Spielordner kopieren",
	"Faltan archivos junto al juego: copia toda la carpeta del juego",
	"File mancanti accanto al gioco: copia l'intera cartella del gioco",
];

/// [S3] The row's note while the card runs no DLSS frame generation (in place of "Applies after a restart").
pub const NO_CARD_NOTE: crate::ui::T5 = [
	"Not supported on this graphics card",
	"Non pris en charge par cette carte graphique",
	"Von dieser Grafikkarte nicht unterst\u{fc}tzt",
	"No compatible con esta tarjeta gr\u{e1}fica",
	"Non supportato da questa scheda grafica",
];

/// The row value of a file value (case and spaces ignored); missing or unknown = Off.
pub fn index(v: Option<&str>) -> u8 {
	v.and_then(|v| KINDS.iter().position(|n| n.eq_ignore_ascii_case(v.trim()))).map_or(0, |i| i as u8)
}

/// Start-up: the choice from the options file, and which values this PC offers.
pub fn init(opts: &PcOptions) {
	let v = index(opts.extra_get(KEY));
	CHOSEN.store(v, Ordering::Relaxed);
	STARTED.store(v, Ordering::Relaxed);
	// lane diag: say WHY a value is missing when its files are not beside the exe (not "cannot run on this PC").
	if cfg!(windows) && !fsr_dlls_there() {
		eprintln!("fgen: FSR frame generation: files missing next to the game (amd_fidelityfx_loader_*.dll + amd_fidelityfx_framegeneration_*.dll): copy the whole game folder");
	}
	if cfg!(windows) && nvidia_there() && !dlss_dlls_there() {
		eprintln!("fgen: DLSS frame generation: files missing next to the game (sl.*.dll, nvngx_dlssg.dll): copy the whole game folder");
	}
	set_runs(FSR_VALUE, fsr_dlls_there() && REFUSED.load(Ordering::Relaxed) & 1 << FSR_VALUE == 0);
	// Only a memory of the card the game will use, on its driver of today (`card_seen` checks it again at the window's device).
	#[cfg(windows)]
	if let (Some(m), Some(c)) = (read_card(), super::sl_card::preferred()) {
		if m.is(&c.name, &c.driver) {
			CARD_MAX.store(m.max, Ordering::Relaxed);
		}
	}
	offer_dlss();
	if v != 0 {
		eprintln!("fgen: Frame Generation {} (options file)", KINDS[v as usize]);
	}
}

/// Whether one backend's FidelityFX frame-generation DLL pair is next to the exe (not on Linux: withdrawn there).
fn fsr_dlls_there() -> bool {
	#[cfg(windows)]
	{
		let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));
		dir.is_some_and(|d| FSR_DLLS.iter().any(|pair| pair.iter().all(|n| d.join(n).is_file())))
	}
	#[cfg(not(windows))]
	false
}

/// [S3] Whether Streamline's DLLs and NVIDIA's frame-generation DLL are all next to the exe (Windows only).
fn dlss_dlls_there() -> bool {
	#[cfg(windows)]
	{
		let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));
		dir.is_some_and(|d| super::sl::SL_DLLS.iter().all(|n| d.join(n).is_file()))
	}
	#[cfg(not(windows))]
	false
}

/// [S3] Whether this PC has an NVIDIA card at all (DXGI, no device made; Windows only).
fn nvidia_there() -> bool {
	#[cfg(windows)]
	return super::sl_card::nvidia_present();
	#[cfg(not(windows))]
	false
}

/// [S3] Offers the DLSS values the DLLs, an NVIDIA card, this run's refusals and the card's remembered maximum allow; the row's
/// note says when the card runs none.
fn offer_dlss() {
	let dlss = dlss_dlls_there() && nvidia_there();
	let max = CARD_MAX.load(Ordering::Relaxed);
	for v in DLSS_VALUES {
		set_runs(v, dlss && REFUSED.load(Ordering::Relaxed) & 1 << v == 0 && dlss_fits(v, max));
	}
	super::follow::refresh(); // lane fglive: the offers and the note follow the Upscaler too (follow.rs)
}

/// Whether this PC's card runs no DLSS frame generation (lane fglive: the row's "Not supported" note, follow.rs `note`).
pub(crate) fn no_card() -> bool {
	dlss_dlls_there() && nvidia_there() && CARD_MAX.load(Ordering::Relaxed) == 0
}

/// Whether frame-generation files are missing next to the exe (the row's "Files missing" note).
pub(crate) fn missing() -> bool {
	cfg!(windows) && (!fsr_dlls_there() || nvidia_there() && !dlss_dlls_there())
}

/// The chosen row value as stored (lane fglive: follow.rs maps it onto the Upscaler and the PC).
pub fn chosen_raw() -> u8 {
	CHOSEN.load(Ordering::Relaxed)
}

/// The row's run-time note: the card cannot (`no_card`) before files are missing (`missing`), else the static note (pure).
pub(crate) fn note_for(no_card: bool, missing: bool) -> Option<&'static crate::ui::T5> {
	if no_card {
		Some(&NO_CARD_NOTE)
	} else if missing {
		Some(&MISSING_NOTE)
	} else {
		None
	}
}

/// [S3] Whether DLSS row value `v` (2..4) fits a card running `max` generated frames (pure).
pub(crate) fn dlss_fits(v: i32, max: u8) -> bool {
	v - 1 <= max as i32
}

/// [S3] The card memory in the card file's text (pure; the max clamped to 3 = 4x). A file of the first format (no driver) reads
/// with an empty driver: it matches no real driver, so it is forgotten at the next device. None: no valid memory.
pub(crate) fn parse_card(text: &str) -> Option<CardMemory> {
	let mut it = text.trim_end_matches(['\r', '\n']).split('\t');
	let name = it.next().filter(|n| !n.is_empty())?.to_string();
	let max = it.next()?.trim().parse::<u8>().ok()?.min(3);
	let driver = it.next().unwrap_or("").trim().to_string();
	Some(CardMemory { name, max, driver })
}

/// [S3] The card file's text of a memory (pure; `parse_card` reads it back).
pub(crate) fn format_card(m: &CardMemory) -> String {
	format!("{}\t{}\t{}\n", m.name, m.max, m.driver)
}

fn read_card() -> Option<CardMemory> {
	parse_card(&std::fs::read_to_string(crate::pcopts::player_dir().join(CARD_FILE)).ok()?)
}

/// [S3] The remembered card, if any (sl_vk.rs `wanted`, before the options are read; `sl_card::vk_gate` verifies it).
pub(crate) fn card_memory() -> Option<CardMemory> {
	read_card()
}

/// [S3] The window's device is made on adapter `name` with driver `driver` (fgen/present.rs `Screen::open`, every start; no
/// Streamline call): a memory of another card or another driver is forgotten (its DLSS values offered again: a driver update may
/// have lifted a refusal); a memory of this card and driver applies.
pub fn card_seen(name: &str, driver: &str) {
	let Some(m) = read_card() else { return };
	if m.is(name, driver) {
		if CARD_MAX.swap(m.max, Ordering::Relaxed) != m.max {
			offer_dlss();
		}
		return;
	}
	let _ = std::fs::remove_file(crate::pcopts::player_dir().join(CARD_FILE));
	eprintln!("fgen: another graphics card or driver than the remembered one ({} {}): DLSS frame generation offered again", m.name, m.driver);
	CARD_MAX.store(3, Ordering::Relaxed);
	offer_dlss();
}

/// [S3] Streamline found out what card `name` on driver `driver` runs (fgen/sl.rs: `max` generated frames, 0 = no DLSS frame
/// generation on it): remembered for the next starts with this driver; the menus offer accordingly from now on.
pub fn remember_card(name: &str, driver: &str, max: u32) {
	let m = CardMemory { name: name.to_string(), max: max.min(3) as u8, driver: driver.to_string() };
	CARD_MAX.store(m.max, Ordering::Relaxed);
	let path = crate::pcopts::player_dir().join(CARD_FILE);
	if read_card().as_ref() != Some(&m) {
		if let Err(e) = std::fs::write(&path, format_card(&m)) {
			eprintln!("fgen: {}: {e}", path.display());
		}
	}
	offer_dlss();
}

/// [S3] The DLSS row values above `frames` generated frames (the card's maximum) cannot run: withdrawn for the run (0 = all).
pub fn refuse_dlss(frames: u32) {
	for v in DLSS_VALUES {
		if v as u32 - 1 > frames {
			refuse(v);
		}
	}
}

/// What the row shows for a stored value `v` (pure): `v` when offered; a DLSS value the card cannot run maps to the highest
/// offered DLSS value below it (that is what runs: sl.rs clamps the multiplier to the card's maximum); anything else Off.
pub(crate) fn shown(v: u8, offered: impl Fn(i32) -> bool) -> u8 {
	let v = v as i32;
	if offered(v) {
		return v as u8;
	}
	if DLSS_VALUES.contains(&v) {
		if let Some(&w) = DLSS_VALUES.iter().rev().find(|&&w| w < v && offered(w)) {
			return w as u8;
		}
	}
	0
}

/// The chosen row value (what the row shows; `shown`: a DLSS multiplier above the card's maximum shows the one that runs, Off
/// while nothing of it can run here).
pub fn chosen() -> u8 {
	shown(CHOSEN.load(Ordering::Relaxed), offered)
}

/// The value this run started with (`shown` as `chosen`): what is in force until a restart.
pub fn started() -> u8 {
	shown(STARTED.load(Ordering::Relaxed), offered)
}

/// Whether the chosen value differs from the one this run started with (the classic row's "- restart").
pub fn restart_pending() -> bool {
	chosen() != started()
}

/// Whether row value `v` can be chosen on this PC: Off always, the others unless the engine found they cannot run.
pub fn offered(v: i32) -> bool {
	v <= 0 || (v as usize) < KINDS.len() && NOT_RUNNING.load(Ordering::Relaxed) & (1 << v.clamp(0, 7)) == 0
}

/// Whether any value but Off can be chosen (no: the classic row reads "Not available").
pub fn available() -> bool {
	(1..KINDS.len() as i32).any(offered)
}

/// The engine found out whether row value `v` runs on this PC: the menus offer it or not. One log line per change. Public for the
/// game packs' menu tests (and S3's DLSS probe).
pub fn set_runs(v: i32, runs: bool) {
	let bit = 1u8 << v.clamp(0, 7);
	let before = if runs { NOT_RUNNING.fetch_and(!bit, Ordering::Relaxed) } else { NOT_RUNNING.fetch_or(bit, Ordering::Relaxed) };
	if (before & bit == 0) == runs {
		return;
	}
	super::follow::refresh(); // lane fglive: the menus' offers = what runs here AND fits the Upscaler (ui/live.rs `offer`)
	let name = KINDS.get(v.max(0) as usize).copied().unwrap_or("?");
	eprintln!("fgen: Frame Generation {name} {}", if runs { "offered" } else { "not offered (files missing or refused: see the fgen lines above)" });
}

/// The engine could not run value `v` this run (no runtime, no queue, a refused swapchain or context): withdrawn for the rest of
/// the run, also when the options are loaded after it (`init`).
pub fn refuse(v: i32) {
	REFUSED.fetch_or(1u8 << v.clamp(0, 7), Ordering::Relaxed);
	set_runs(v, false);
}

/// The menu set the row (0 Off, 1 FSR 2x, 2..4 DLSS 2x..4x): remembered in the options file; in force at once when the running
/// maker makes it, else from the next start (lane fglive, follow.rs); true = the file changed.
pub fn set(opts: &mut PcOptions, v: i32) -> bool {
	let v = v.clamp(0, KINDS.len() as i32 - 1) as u8;
	if CHOSEN.swap(v, Ordering::Relaxed) == v {
		return false;
	}
	opts.extra_set(KEY, Some(KINDS[v as usize].to_string()));
	eprintln!("fgen: PC options: {KEY} {}", KINDS[v as usize]);
	super::follow::refresh(); // one line: in force, paused or not, live or after a restart
	true
}

/// The options file's choice as the `RAC_FG` value it stands for (`mod.rs` `mode`), read before the window opens.
pub(crate) fn file_value() -> Option<&'static str> {
	let opts = PcOptions::load(&crate::pcopts::file());
	match index(opts.extra_get(KEY)) {
		0 => None,
		v => Some(KINDS[v as usize]),
	}
}

#[cfg(test)]
pub(crate) fn reset_for_test() {
	CHOSEN.store(0, Ordering::Relaxed);
	STARTED.store(0, Ordering::Relaxed);
	NOT_RUNNING.store(0, Ordering::Relaxed);
	REFUSED.store(0, Ordering::Relaxed);
	CARD_MAX.store(3, Ordering::Relaxed);
	super::follow::reset_for_test();
	crate::ui::live::set_note(KEY, None);
	crate::ui::live::offer(KEY, FSR_VALUE, true);
	for v in DLSS_VALUES {
		crate::ui::live::offer(KEY, v, true);
	}
}
