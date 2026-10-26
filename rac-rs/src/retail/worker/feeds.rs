//! `load`, part 3: the travel request, the channels to the worker thread, the recorded pad (`@play`) and the title boot.
#![allow(unused_mut)]

use super::*;

pub(super) fn travel_request(replay: &Option<Arc<rec::Replay>>) -> Option<(u32, u32)> {
	// RAC_RETAIL_TRAVEL=LEVEL:FIELD (shots and tests): at that field, request travel the way the level-end helper
	// does (Eudora 0x27f6d0: target 0x15f5a0, 0x15f5b8 = 1, 0x15f550 = 1).
	let travel: Option<(u32, u32)> = std::env::var("RAC_RETAIL_TRAVEL").ok().and_then(|v| {
		let (l, f) = v.split_once(':')?;
		Some((l.parse().ok()?, f.parse().ok()?))
	});
	// A replayed file's last travel request stands in for RAC_RETAIL_TRAVEL (shots wait for that level).
	let travel = match &replay {
		Some(r) => r.fields.iter().enumerate().rev().find_map(|(i, f)| Some((f.travel?, i as u32))),
		None => travel,
	};
	travel
}

pub(super) struct Feeds {
	pub(super) tx: std::sync::mpsc::Sender<FieldIn>,
	pub(super) prx: std::sync::mpsc::Receiver<FieldIn>,
	pub(super) play: Option<playfeed::Feed>,
	pub(super) ptx: std::sync::mpsc::Sender<Snap>,
	pub(super) rx: std::sync::mpsc::Receiver<Snap>,
	pub(super) gsh: Option<Arc<crate::gsfull::pipe::GsFull>>,
	pub(super) tb: Option<title_boot::Boot>,
	pub(super) hold_card: bool,
	pub(super) booting: bool,
	pub(super) cold_front: bool,
}

pub(super) fn open_feeds(dir: &String, cold: Option<&String>, gshared: &Option<Arc<crate::gsfull::pipe::GsFull>>) -> Feeds {
	let (tx, prx) = std::sync::mpsc::channel::<FieldIn>();
	// RAC_RETAIL_PLAY=@play (acceptance test): the recorded pad report + RAM pokes of the savestate dir's pad.raw / pad.pokes.
	let play = playfeed::open(std::env::var("RAC_RETAIL_PLAY").ok().as_deref(), false, dir, 0, "RAC_RETAIL_PLAYTRAIL", playfeed::code_hash(include_str!("../../../../oracle/pcsx2rec/playfeed.rs")));
	let (ptx, rx) = std::sync::mpsc::channel();
	let gsh = gshared.clone();
	let tb = if title_boot::enabled() { Some(title_boot::Boot::new()) } else if cold.is_some() && std::env::var_os("RAC_TITLE_PILOT").is_some() { Some(title_boot::Boot::pilot_only()) } else { None };
	let hold_card = title_boot::enabled();
	// Cold boot: the logos run in the boot ELF before the front end is in RAM; the worker sends the blank front snapshot until then.
	let mut booting = cold.is_some();
	// ... and the boot ELF's own front end (title, New Game) is "the front end" until the first level overlay is in RAM
	// (its movie player is not frontbin's: `title_boot::front_end` never matches there).
	let mut cold_front = cold.is_some();
	Feeds { tx, prx, play, ptx, rx, gsh, tb, hold_card, booting, cold_front }
}
