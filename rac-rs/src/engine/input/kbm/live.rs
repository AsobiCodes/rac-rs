//! The live state the Controls page and the input share: the bindings, the controller map, the change flag and the capture state
//! machine (keyboard + mouse slots and the controller, one machine).

use super::binds::Binds;
use super::file;
use super::names::capturable;
use super::types::*;
use super::padfile;
use crate::input::binds::{NP, PHYS};

/// What the Controls page waits for: the next fresh input for slot `1` of action `0` (`Kbm`) or the next controller button for slot `1`
/// of action `0` (`Pad`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cap {
	Idle,
	Kbm(usize, usize),
	Pad(usize, usize),
}

/// Fields (one per `tick_keys`, 20 ms) a capture waits before it gives up: 8 s.
pub const CAP_FIELDS: u32 = 400;

struct Live {
	table: Option<&'static Table>,
	binds: Option<Binds>,
	/// A binding changed since the worker last looked (it writes the options file).
	dirty: bool,
	cap: Cap,
	fields: u32,
	ended: bool,
	/// The texts `init` read: they are read again when a different table is set afterwards (`init` may run before the game's glue
	/// has set its table; the bindings of the pack's table must not be thrown away unread).
	init_args: Option<[Option<String>; 4]>,
	/// Whether any controller slot differs from the defaults (cached; any change of the bindings clears it).
	pad_remapped: Option<bool>,
}

impl Live {
	const START: Live = Live { table: None, binds: None, dirty: false, cap: Cap::Idle, fields: 0, ended: false, init_args: None, pad_remapped: None };

	fn t(&self) -> &'static Table {
		self.table.unwrap_or_else(|| crate::pack::get().kbm)
	}

	/// `gamepad` wins over an old `pad` line, as `kbm` over `keys`.
	fn load(&mut self, kbm: Option<&str>, keys: Option<&str>, pad: Option<&str>, gamepad: Option<&str>) {
		let t = self.t();
		let mut b = match (kbm, keys) {
			(Some(s), _) => file::from_text(t, s),
			(None, Some(k)) => file::convert_old(t, k),
			_ => Binds::defaults(t),
		};
		b.1 = match (gamepad, pad) {
			(Some(s), _) => padfile::from_text(t, s),
			(None, Some(p)) => padfile::convert_old(t, p),
			_ => b.1,
		};
		self.binds = Some(b);
		self.dirty = false;
		self.pad_remapped = None;
	}

	/// The bindings for changing (clears the cached `pad_remapped`).
	fn b(&mut self) -> &mut Binds {
		self.pad_remapped = None;
		self.r();
		self.binds.as_mut().expect("set by r")
	}

	/// The bindings for reading.
	fn r(&mut self) -> &Binds {
		let t = self.t();
		self.binds.get_or_insert_with(|| Binds::defaults(t))
	}

	fn assign(&mut self, a: usize, slot: usize, i: Input) {
		if a < self.t().actions.len() && slot < 2 && capturable(i) {
			self.b().assign(a, slot, i);
			self.dirty = true;
		}
	}

	fn assign_pad(&mut self, a: usize, slot: usize, p: usize) {
		let t = self.t();
		if p < NP && self.b().assign_pad(t, a, slot, p as PadIn) {
			self.dirty = true;
		}
	}
}

#[cfg(not(test))]
fn live<R>(f: impl FnOnce(&mut Live) -> R) -> R {
	static L: std::sync::Mutex<Live> = std::sync::Mutex::new(Live::START);
	f(&mut L.lock().unwrap_or_else(|e| e.into_inner()))
}
#[cfg(test)]
fn live<R>(f: impl FnOnce(&mut Live) -> R) -> R {
	thread_local! { static L: std::cell::RefCell<Live> = const { std::cell::RefCell::new(Live::START) }; }
	L.with(|l| f(&mut l.borrow_mut()))
}

/// The action table in force: the one `ensure_table` set, else the game pack's.
pub fn table() -> &'static Table {
	live(|l| l.t())
}

/// Sets the table when none was set yet (a game's glue calls this before it reads the bindings).
pub fn ensure_table(t: &'static Table) {
	live(|l| {
		if l.table.is_none() {
			// Binds read for the pack's own table (`init` ran first) stay; only a different table starts over.
			let same = std::ptr::eq(t, crate::pack::get().kbm);
			l.table = Some(t);
			if !same {
				l.binds = None;
				l.pad_remapped = None;
				if let Some([k, o, p, g]) = l.init_args.clone() {
					l.load(k.as_deref(), o.as_deref(), p.as_deref(), g.as_deref());
				}
			}
		}
	});
}

pub fn binds() -> Binds {
	live(|l| *l.r())
}

/// Whether any controller slot differs from the defaults of the table (cached per change; called for every text lookup).
pub fn pad_remapped() -> bool {
	live(|l| {
		if let Some(r) = l.pad_remapped {
			return r;
		}
		let t = l.t();
		let r = l.r().1 != Binds::defaults(t).1;
		l.pad_remapped = Some(r);
		r
	})
}

/// The old 16-function controller view (function -> `PHYS` index of its action's first button): the classic Controls page and the
/// glyphs until they read the actions (stage 2).
pub fn pad_map() -> [u8; NP] {
	live(|l| {
		let t = l.t();
		padfile::legacy_map(t, l.r())
	})
}

/// Startup: the options file's `kbm`, old `keys`, old `pad` and `gamepad` values (`kbm` wins over `keys`, `gamepad` over `pad`; the
/// old lines are converted).
pub fn init(kbm: Option<&str>, keys: Option<&str>, pad: Option<&str>, gamepad: Option<&str>) {
	live(|l| {
		l.init_args = Some([kbm, keys, pad, gamepad].map(|o| o.map(str::to_owned)));
		l.load(kbm, keys, pad, gamepad);
	});
}

pub fn assign(action: usize, slot: usize, input: Input) {
	live(|l| l.assign(action, slot, input));
}

/// Empties a slot (refused for a `keep_one` action's last binding); true when it changed.
pub fn clear(action: usize, slot: usize) -> bool {
	live(|l| {
		let t = l.t();
		let ok = l.b().clear(t, action, slot);
		l.dirty |= ok;
		ok
	})
}

/// Controller slot `slot` of `action` gets the button `p` (a `PHYS` index).
pub fn assign_pad(action: usize, slot: usize, p: usize) {
	live(|l| l.assign_pad(action, slot, p));
}

/// Empties a controller slot (refused for a `keep_one` action's last button); true when it changed.
pub fn clear_pad(action: usize, slot: usize) -> bool {
	live(|l| {
		let t = l.t();
		let ok = l.b().clear_pad(t, action, slot);
		l.dirty |= ok;
		ok
	})
}

pub fn reset_kbm() {
	live(|l| {
		let t = l.t();
		l.b().reset_kbm(t);
		l.dirty = true;
	});
}

pub fn reset_pad() {
	live(|l| {
		let t = l.t();
		l.b().reset_pad(t);
		l.dirty = true;
	});
}

/// Worker: once after a change, the `kbm` value and the `gamepad` value to write (None = the defaults: the line goes).
pub fn take_changes() -> Option<(Option<String>, Option<String>)> {
	live(|l| {
		let t = l.t();
		std::mem::take(&mut l.dirty).then(|| (file::to_text(t, l.b()), padfile::to_text(t, l.b())))
	})
}

/// Worker: like `take_changes`, but the first value is the old 24-function `keys=` text of the keys alone (the classic page's tests)
/// and the second the `gamepad` value.
pub fn take_changes_legacy() -> Option<(Option<String>, Option<String>)> {
	live(|l| {
		let t = l.t();
		std::mem::take(&mut l.dirty).then(|| (file::legacy_view(t, l.b()).to_text(), padfile::to_text(t, l.b())))
	})
}

pub fn capture_begin(c: Cap) {
	live(|l| {
		l.cap = c;
		l.fields = 0;
		l.ended = false;
	});
}

pub fn capture_cancel() {
	live(|l| {
		if l.cap != Cap::Idle {
			l.cap = Cap::Idle;
			l.ended = true;
		}
	});
}

/// What the capture waits for now (`Cap::Idle` = none): the modern Controls table marks that cell.
pub fn capture_state() -> Cap {
	live(|l| l.cap)
}

/// A capture is waiting, so the game gets a neutral pad this field.
pub fn capturing() -> bool {
	live(|l| l.cap != Cap::Idle)
}

/// True once when the capture ended (bound, cancelled or timed out).
pub fn capture_ended() -> bool {
	live(|l| std::mem::take(&mut l.ended))
}

/// Every field while `capturing`: `fresh` are the inputs that went down this field (taps at their first poll, so the press that
/// started the capture never binds itself), `phys` the controller buttons that went down. Binds the first capturable one; Esc or the
/// time limit ends it without a binding.
pub fn capture_poll(fresh: &[Input], phys: u32) {
	live(|l| {
		if l.cap == Cap::Idle {
			return;
		}
		l.fields += 1;
		let esc = fresh.contains(&Input::Key(winit::keyboard::KeyCode::Escape));
		let done = if esc || l.fields > CAP_FIELDS {
			true
		} else {
			match l.cap {
				Cap::Kbm(a, s) => fresh.iter().copied().find(|&i| capturable(i)).map(|i| l.assign(a, s, i)).is_some(),
				// The first poll after the begin is skipped: the press that opened the capture (a pad's Cross on the Controls page) can still
				// look fresh to the poll that follows the begin (the begin and the poll run on different threads).
				Cap::Pad(a, s) => l.fields > 1 && (0..NP).find(|&p| phys & PHYS[p].1 != 0).map(|p| l.assign_pad(a, s, p)).is_some(),
				Cap::Idle => false,
			}
		};
		if done {
			l.cap = Cap::Idle;
			l.ended = true;
		}
	});
}
