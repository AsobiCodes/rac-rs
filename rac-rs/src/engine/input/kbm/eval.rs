//! Evaluation: held inputs -> pad buttons, sticks and PC actions, layer by layer.

use super::binds::Binds;
use super::types::*;

/// What is active this field.
#[derive(Clone, Copy, Default, Debug)]
pub struct Ctx {
	/// Bit `n` set = `Layer::Context(n)` is active.
	pub contexts: u32,
	/// A menu screen is up: the Menu layer is on.
	pub menu: Option<MenuCtx>,
}

/// The result: pad buttons, sticks `[rx, ry, lx, ly]` and the bits of the PC actions held (bit `1 << id`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Out {
	pub buttons: u16,
	pub sticks: [u8; 4],
	pub pc: u32,
}

/// Evaluates `binds` under `table`. `held` says whether an input is down (taps and mouse state are the host's business).
pub fn eval(t: &Table, binds: &Binds, held: &dyn Fn(Input) -> bool, ctx: &Ctx) -> Out {
	let n = t.actions.len().min(MAX_ACTIONS);
	let bound = |a: usize| binds.0[a].iter().flatten().copied();
	// Shadows: the inputs a higher layer consumes. `stick_shadow` is the Menu layer's smaller set for the sticks.
	let mut shadow: Vec<Input> = Vec::new();
	let mut stick_shadow: Vec<Input> = Vec::new();
	if ctx.menu.is_some() {
		shadow.extend(t.menu.iter().map(|m| m.input.norm()));
		stick_shadow.extend(t.menu.iter().filter(|m| m.stick_shadow).map(|m| m.input.norm()));
	}
	let active = |l: Layer| match l {
		Layer::Game => true,
		Layer::Context(c) => c < 32 && ctx.contexts & (1 << c) != 0,
	};
	for a in (0..n).filter(|&a| matches!(t.actions[a].layer, Layer::Context(_)) && active(t.actions[a].layer)) {
		let v: Vec<Input> = bound(a).collect();
		shadow.extend(v.iter());
		stick_shadow.extend(v);
	}
	let mut buttons = 0u16;
	let mut pc = 0u32;
	let mut d = [0i32; 4];
	for a in (0..n).filter(|&a| active(t.actions[a].layer)) {
		let def = &t.actions[a];
		let is_game = def.layer == Layer::Game;
		let on_in = |sh: &[Input]| bound(a).any(|i| (!is_game || !sh.contains(&i)) && i.is_down(held));
		match def.output {
			Output::Pad(bit) => {
				if on_in(&shadow) {
					buttons |= bit;
				}
			}
			Output::Stick(axis, dir) => {
				if ctx.menu.map_or(true, |m| m.sticks) && on_in(&stick_shadow) {
					d[axis as usize & 3] += dir as i32;
				}
			}
			Output::Pc(id) => {
				if on_in(&shadow) {
					pc |= 1 << (id & 31);
				}
			}
		}
	}
	if let Some(m) = ctx.menu {
		for k in t.menu {
			if (k.unless_sticks && m.sticks) || !k.input.is_down(held) {
				continue;
			}
			buttons |= match k.out {
				MenuOut::None => 0,
				MenuOut::Pad(b) => b,
				MenuOut::Confirm => m.confirm,
				MenuOut::Back => m.back,
				MenuOut::Alt => m.alt,
			};
		}
	}
	Out { buttons, sticks: d.map(|d| (0x80 + d * 0x7f).clamp(0, 0xff) as u8), pc }
}
