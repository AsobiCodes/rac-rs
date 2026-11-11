//! The controller in the options file: `gamepad=jump:b|-,aim:lt|rt` (action id, `|` between the two slots, `-` empty, button names =
//! `binds::PHYS` file names; only actions that differ from the defaults; unreadable entries skipped), the conversion of an old
//! `pad=cross:b,l2:lb` line (function -> button, v0.4.1..v0.5.0) and the old 16-function view the classic Controls page still uses.

use super::binds::Binds;
use super::file::func_action;
use super::types::*;
use crate::input::binds::{pad_from_text, BIT, DEFAULT_PAD, NP, PHYS};

pub type PadSlots = [[Option<PadIn>; 2]; MAX_ACTIONS];

fn name(p: Option<PadIn>) -> &'static str {
	p.map_or("-", |p| PHYS[p as usize % PAD_BUTTONS].0)
}

fn from_name(s: &str) -> Option<PadIn> {
	PHYS.iter().position(|e| e.0.eq_ignore_ascii_case(s.trim())).map(|p| p as PadIn)
}

/// `gamepad=` value: the actions that differ from the defaults; None when nothing differs.
pub fn to_text(t: &Table, b: &Binds) -> Option<String> {
	let d = Binds::defaults(t);
	let v: Vec<String> = (0..t.actions.len().min(MAX_ACTIONS)).filter(|&a| t.actions[a].pad_ok && b.1[a] != d.1[a]).map(|a| format!("{}:{}|{}", t.actions[a].id, name(b.1[a][0]), name(b.1[a][1]))).collect();
	(!v.is_empty()).then(|| v.join(","))
}

/// The controller slots a `gamepad=` value says: the defaults with every readable entry applied. An entry with an unreadable button
/// name, for an action the controller may not bind, or one that would leave a `keep_one` action without a button is skipped.
pub fn from_text(t: &Table, s: &str) -> PadSlots {
	let mut b = Binds::defaults(t);
	for e in s.split(',') {
		let Some((id, v)) = e.split_once(':') else { continue };
		let Some(a) = t.index(id).filter(|&a| a < MAX_ACTIONS && t.actions[a].pad_ok) else { continue };
		let parts: Vec<&str> = v.split('|').map(str::trim).collect();
		if parts.is_empty() || parts.len() > 2 {
			continue;
		}
		let mut slots = [None; 2];
		let mut ok = true;
		for (i, p) in parts.iter().enumerate() {
			if !(p.is_empty() || *p == "-") {
				match from_name(p) {
					Some(x) => slots[i] = Some(x),
					None => ok = false,
				}
			}
		}
		if ok && !(t.actions[a].keep_one && slots == [None; 2]) {
			b.1[a] = slots;
		}
	}
	b.1
}

/// An old `pad=` value as controller slots: function `f`'s button goes to slot 1 of the action that drives it; an action that follows
/// a function (`pad_follows`: the Aim follows the old R2 button) takes that function's button too, so the aim stays where it was.
pub fn convert_old(t: &Table, pad: &str) -> PadSlots {
	let old = pad_from_text(pad);
	let mut b = Binds::defaults(t);
	for f in (0..NP).filter(|&f| old[f] != DEFAULT_PAD[f]) {
		if let Some(a) = func_action(t, f).filter(|&a| a < MAX_ACTIONS && t.actions[a].pad_ok) {
			b.1[a] = [Some(old[f]), None];
		}
		for a in (0..t.actions.len().min(MAX_ACTIONS)).filter(|&a| t.actions[a].pad_ok && t.actions[a].pad_follows == BIT[f]) {
			b.1[a] = [Some(old[f]), None];
		}
	}
	b.1
}

/// The old 16-function view: per function the first button of its action (the default's when the action has none).
pub fn legacy_map(t: &Table, b: &Binds) -> [u8; NP] {
	let mut m = DEFAULT_PAD;
	for (f, e) in m.iter_mut().enumerate() {
		if let Some(a) = func_action(t, f).filter(|&a| a < MAX_ACTIONS) {
			if let Some(p) = b.1[a].iter().flatten().next() {
				*e = *p;
			}
		}
	}
	m
}

/// The action the old page's controller row `f` edits, and the slot it writes (the first with a button, else the first).
pub fn legacy_slot(t: &Table, b: &Binds, f: usize) -> Option<(usize, usize)> {
	let a = func_action(t, f).filter(|&a| a < MAX_ACTIONS && t.actions[a].pad_ok)?;
	Some((a, (0..2).find(|&s| b.1[a][s].is_some()).unwrap_or(0)))
}
