//! The options file: `kbm=` read / write, the conversion of an old `keys=` line, and the old 24-function view of the bindings that
//! the classic Controls page still uses (stage 1 adapter).

use super::binds::Binds;
use super::names::{file_name, from_name};
use super::types::*;
use crate::input::binds::{KbBinds, BIT, DEFAULT_KEYS, NK, STICK};
use winit::keyboard::KeyCode as K;

/// The output of PS2 function `f` (`binds::ID` order).
pub fn func_output(f: usize) -> Output {
	if f < 16 {
		Output::Pad(BIT[f])
	} else {
		let (axis, dir) = STICK[f - 16];
		Output::Stick(axis as u8, dir as i8)
	}
}

/// The Game-layer action that drives PS2 function `f`.
pub fn func_action(t: &Table, f: usize) -> Option<usize> {
	t.game_action(func_output(f))
}

/// `kbm=` value: the actions that differ from the defaults, `id:slot1|slot2` (`-` = empty); None when nothing differs.
pub fn to_text(t: &Table, b: &Binds) -> Option<String> {
	let d = Binds::defaults(t);
	let one = |i: Option<Input>| i.and_then(file_name).unwrap_or("-");
	let v: Vec<String> = (0..t.actions.len().min(MAX_ACTIONS)).filter(|&a| b.0[a] != d.0[a]).map(|a| format!("{}:{}|{}", t.actions[a].id, one(b.0[a][0]), one(b.0[a][1]))).collect();
	(!v.is_empty()).then(|| v.join(","))
}

/// The bindings a `kbm=` value says: the defaults with every readable entry applied. An entry with an unreadable name, or one that
/// would leave a `keep_one` action empty, is skipped.
pub fn from_text(t: &Table, s: &str) -> Binds {
	let mut b = Binds::defaults(t);
	for e in s.split(',') {
		let Some((id, v)) = e.split_once(':') else { continue };
		let Some(a) = t.index(id).filter(|&a| a < MAX_ACTIONS) else { continue };
		let mut slots = [None; 2];
		let parts: Vec<&str> = v.split('|').map(str::trim).collect();
		if parts.is_empty() || parts.len() > 2 {
			continue;
		}
		let mut ok = true;
		for (i, p) in parts.iter().enumerate() {
			if *p == "-" || p.is_empty() {
				continue;
			}
			match from_name(p) {
				Some(inp) => slots[i] = Some(inp),
				None => ok = false,
			}
		}
		if ok && !(t.actions[a].keep_one && slots == [None; 2]) {
			b.0[a] = slots;
		}
	}
	b
}

/// An old `keys=` value (v0.4.1..v0.5.0) as bindings: per function the old keys go to the first slots, the mouse button the
/// function has by default (old files could not move it) to the slot after them. A second hand-edited key of a function that keeps a
/// mouse button is dropped (the Controls page only ever wrote one key).
pub fn convert_old(t: &Table, keys: &str) -> Binds {
	let old = KbBinds::from_text(keys);
	let mut b = Binds::defaults(t);
	for f in (0..NK).filter(|&f| old.0[f] != DEFAULT_KEYS[f]) {
		let Some(a) = func_action(t, f).filter(|&a| a < MAX_ACTIONS) else { continue };
		let mut keys: Vec<Input> = Vec::new();
		for k in old.0[f].iter().flatten() {
			let i = Input::Key(*k).norm();
			if !keys.contains(&i) {
				keys.push(i);
			}
		}
		// The old keys take the first slots, a mouse default the rest (so a remapped Circle reads "H" in the prompts as in v0.5.0 and
		// the old fixed LMB keeps firing in slot 2). A function with no mouse default has two key slots. A second hand-edited key of
		// a function that keeps a mouse button is dropped (the Controls page only ever wrote one key).
		let mice: Vec<Option<Input>> = t.actions[a].defaults.iter().copied().filter(|d| matches!(d, Some(Input::Mouse(_)) | Some(Input::Wheel(_)))).collect();
		let mut it = keys.into_iter();
		let mut slots = [None; 2];
		let nk = 2 - mice.len().min(2);
		for (s, k) in slots.iter_mut().zip(it.by_ref().take(nk)) {
			*s = Some(k);
		}
		for (n, m) in mice.into_iter().take(2).enumerate() {
			slots[nk + n] = m;
		}
		b.0[a] = slots;
	}
	b
}

/// The old 24-function view: per function the keys (not mouse inputs) of its action; Pause shows its Enter as Enter + Numpad Enter.
pub fn legacy_view(t: &Table, b: &Binds) -> KbBinds {
	let mut o = KbBinds::default();
	for f in 0..NK {
		let Some(a) = func_action(t, f).filter(|&a| a < MAX_ACTIONS) else { continue };
		let mut ks = [None; 3];
		let mut n = 0;
		for i in b.0[a].iter().flatten() {
			if let Input::Key(k) = i {
				ks[n] = Some(*k);
				n += 1;
				if *k == K::Enter && DEFAULT_KEYS[f].contains(&Some(K::NumpadEnter)) && n < 3 {
					ks[n] = Some(K::NumpadEnter);
					n += 1;
				}
			}
		}
		o.0[f] = ks;
	}
	o
}

/// The slot the old page's key goes to for function `f`: the first slot holding a key, else the first without a mouse input.
pub fn legacy_slot(t: &Table, b: &Binds, f: usize) -> Option<(usize, usize)> {
	let a = func_action(t, f).filter(|&a| a < MAX_ACTIONS)?;
	let s = (0..2).find(|&s| matches!(b.0[a][s], Some(Input::Key(_)))).or_else(|| (0..2).find(|&s| !matches!(b.0[a][s], Some(Input::Mouse(_)) | Some(Input::Wheel(_)))))?;
	Some((a, s))
}
