//! The bindings: per action two keyboard + mouse slots (`.0`) and two controller slots (`.1`, v0.5.1 pad lane).

use super::types::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Binds(pub [[Option<Input>; 2]; MAX_ACTIONS], pub [[Option<PadIn>; 2]; MAX_ACTIONS]);

impl Binds {
	/// A table's defaults.
	pub fn defaults(t: &Table) -> Binds {
		let mut b = Binds([[None; 2]; MAX_ACTIONS], [[None; 2]; MAX_ACTIONS]);
		for (i, a) in t.actions.iter().enumerate().take(MAX_ACTIONS) {
			b.0[i] = a.defaults.map(|d| d.map(Input::norm));
			if a.pad_ok {
				b.1[i] = a.pad_defaults;
			}
		}
		b
	}

	/// Controller slot `slot` of `action` gets button `p` (a `PHYS` index); refused (false) when the action is not controller-bindable.
	pub fn assign_pad(&mut self, t: &Table, action: usize, slot: usize, p: PadIn) -> bool {
		let ok = action < t.actions.len().min(MAX_ACTIONS) && slot < 2 && t.actions[action].pad_ok && (p as usize) < PAD_BUTTONS;
		if ok {
			self.1[action][slot] = Some(p);
		}
		ok
	}

	/// Empties a controller slot; refused (false) when the action is `keep_one` and this is its last controller button.
	pub fn clear_pad(&mut self, t: &Table, action: usize, slot: usize) -> bool {
		if action >= t.actions.len().min(MAX_ACTIONS) || slot >= 2 || self.1[action][slot].is_none() {
			return false;
		}
		if t.actions[action].keep_one && self.1[action][1 - slot].is_none() {
			return false;
		}
		self.1[action][slot] = None;
		true
	}

	/// Back to the defaults of the keyboard + mouse only; the controller keeps its buttons.
	pub fn reset_kbm(&mut self, t: &Table) {
		self.0 = Binds::defaults(t).0;
	}

	/// Back to the defaults of the controller only.
	pub fn reset_pad(&mut self, t: &Table) {
		self.1 = Binds::defaults(t).1;
	}

	/// Whether `action` has controller button `p` in a slot.
	pub fn has_pad(&self, action: usize, p: PadIn) -> bool {
		action < MAX_ACTIONS && self.1[action].contains(&Some(p))
	}

	/// The other controller-bindable actions that share a button with `action` (the Controls page shows a shared button on every row).
	pub fn shared_pad(&self, t: &Table, action: usize) -> Vec<usize> {
		if action >= t.actions.len().min(MAX_ACTIONS) {
			return Vec::new();
		}
		(0..t.actions.len().min(MAX_ACTIONS)).filter(|&g| g != action && t.actions[g].pad_ok && self.1[g].iter().flatten().any(|p| self.1[action].contains(&Some(*p)))).collect()
	}

	/// Slot `slot` of `action` gets `input`; nothing else moves (an input may drive several actions).
	pub fn assign(&mut self, action: usize, slot: usize, input: Input) {
		if action < MAX_ACTIONS && slot < 2 {
			self.0[action][slot] = Some(input.norm());
		}
	}

	/// Empties the slot; refused (false) when the action is `keep_one` and this is its last binding.
	pub fn clear(&mut self, t: &Table, action: usize, slot: usize) -> bool {
		if action >= t.actions.len().min(MAX_ACTIONS) || slot >= 2 || self.0[action][slot].is_none() {
			return false;
		}
		if t.actions[action].keep_one && self.0[action][1 - slot].is_none() {
			return false;
		}
		self.0[action][slot] = None;
		true
	}

	/// Both devices back to the defaults.
	pub fn reset(&mut self, t: &Table) {
		*self = Binds::defaults(t);
	}

	/// Whether `action` has `input` in a slot.
	pub fn has(&self, action: usize, input: Input) -> bool {
		action < MAX_ACTIONS && self.0[action].contains(&Some(input.norm()))
	}

	/// The other actions of the same layer that share an input with `action` (the Controls page shows a shared input on every row).
	pub fn shared(&self, t: &Table, action: usize) -> Vec<usize> {
		let Some(a) = t.actions.get(action).filter(|_| action < MAX_ACTIONS) else { return Vec::new() };
		(0..t.actions.len().min(MAX_ACTIONS)).filter(|&g| g != action && t.actions[g].layer == a.layer && self.0[g].iter().flatten().any(|i| self.0[action].contains(&Some(*i)))).collect()
	}

	/// The ids of the PC actions `input` is bound to.
	pub fn pc_actions<'a>(&'a self, t: &'a Table, input: Input) -> impl Iterator<Item = u8> + 'a {
		let i = input.norm();
		t.actions.iter().enumerate().take(MAX_ACTIONS).filter_map(move |(n, a)| match a.output {
			Output::Pc(id) if self.0[n].contains(&Some(i)) => Some(id),
			_ => None,
		})
	}
}
