//! Evaluation of the controller (v0.5.1 pad lane, design `V051_PAD_DESIGN.md`): the physical buttons held (`binds::PHYS` masks) ->
//! per Game-layer action whose controller slot is down -> its output (a pad bit, or a PC action). The sticks are not here: they stay
//! analog and fixed (`pad::map_xinput`). The Menu context is not evaluated either: a game reads the controller in the STANDARD layout
//! there, which is `eval_pad` with `Binds::defaults(table)`.

use super::binds::Binds;
use super::types::*;
use crate::input::binds::PHYS;

/// What the controller gives this field.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PadEval {
	/// The PS2 pad bits.
	pub buttons: u16,
	/// The PC actions held (bit `1 << id`).
	pub pc: u32,
	/// `buttons` without the outputs of the inputs a `consume` rule takes (see `eval_pad`); equal to `buttons` when nothing is consumed.
	pub free: u16,
}

fn down(phys: u32, p: PadIn) -> bool {
	phys & PHYS[p as usize % PAD_BUTTONS].1 != 0
}

/// The controller's actions for the physical buttons `phys`. `consume` = the PC action id (R&C1's Aim) whose inputs it consumes while
/// the caller's gate is on (decided 2026-11-05): `free` is `buttons` computed without those inputs, every other output of an Aim
/// button included (a Jump on the same button too); an output another, non-Aim button also gives stays.
pub fn eval_pad(t: &Table, b: &Binds, phys: u32, consume: Option<u8>) -> PadEval {
	let n = t.actions.len().min(MAX_ACTIONS);
	let mut eaten = [false; PAD_BUTTONS];
	if let Some(id) = consume {
		for a in (0..n).filter(|&a| t.actions[a].pad_ok && t.actions[a].output == Output::Pc(id)) {
			for p in b.1[a].iter().flatten() {
				eaten[*p as usize % PAD_BUTTONS] = true;
			}
		}
	}
	let mut o = PadEval::default();
	for a in (0..n).filter(|&a| t.actions[a].pad_ok && t.actions[a].layer == Layer::Game) {
		let slots = || b.1[a].iter().flatten().copied();
		match t.actions[a].output {
			Output::Pad(bit) => {
				if slots().any(|p| down(phys, p)) {
					o.buttons |= bit;
				}
				if slots().any(|p| !eaten[p as usize % PAD_BUTTONS] && down(phys, p)) {
					o.free |= bit;
				}
			}
			Output::Pc(id) => {
				if slots().any(|p| down(phys, p)) {
					o.pc |= 1 << (id & 31);
				}
			}
			Output::Stick(..) => {}
		}
	}
	o
}
