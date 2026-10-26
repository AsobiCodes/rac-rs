//! The one generic hook bridge: every mod hook point is a relocatable hand (oracle `hs_reloc`) whose function is `hook_n::<N>`, N = the
//! binding's hook index. A hand runs at the entry of the retail function with the arguments in the registers (EE EABI: integers in
//! r4..r11 in the order of the integer arguments, floats in f12..f19 in the order of the float arguments, each class counting on its own;
//! integer results in r2 sign-extended, float results in f0). The bridge reads them by the binding's signature, calls
//! `omni_dispatch`, and the mods' chain ends in `original(...)`, which comes back here (hooks.rs `original_cb`) and runs the retail
//! function with `frame_wait::call`.
//!
//! EXACTNESS: with a chain that calls `original` unchanged, game state and instruction count are those of a run without the hook. The
//! hand counts nothing itself (the nested retail call counts its real instructions), arguments and results are written back only where the
//! typed value changed, `r31` is left alone, and the PC is the return address when the hand ends. A nested entry of the same hook
//! (the retail function called by `original`, a recursive call) is not dispatched: the hand returns false and the retail code runs.

use super::binding::{Sig, Ty};
use super::cabi::Value;
use super::hooks::slot;
use ee_oracle::cpu::Cpu;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// One dispatch in progress: what `original_cb` needs to run the retail function for it.
pub struct Ctx {
	pub slot: usize,
	pub entry: u32,
	pub ra: u32,
	/// The chain called `original` at least once.
	pub called: bool,
	/// The retail callee did not come back (it waits for a frame, frame_wait.rs): the hand ends at once, the host loop goes on from the wait.
	pub unwound: bool,
}

pub static CTX: Mutex<Vec<Ctx>> = Mutex::new(Vec::new());

fn ctx() -> std::sync::MutexGuard<'static, Vec<Ctx>> {
	CTX.lock().unwrap_or_else(|e| e.into_inner())
}

/// A hook dispatch is in progress (a `call_game` from inside it behaves like `call_original`; between two fields it does not).
pub fn in_hook() -> bool {
	!ctx().is_empty()
}

/// A dispatch of hook `n` is in progress (its `original` can take the entry and return address from it).
pub fn dispatching(n: usize) -> bool {
	dispatching_in(&ctx(), n)
}

fn dispatching_in(st: &[Ctx], n: usize) -> bool {
	st.iter().any(|x| x.slot == n)
}

/// Re-entrancy guard of one hook (the retail function that `original` calls runs the hand again, which must fall through to retail).
pub struct Guard(&'static AtomicBool);
impl Guard {
	pub fn take(f: &'static AtomicBool) -> Option<Guard> {
		(!f.swap(true, Ordering::SeqCst)).then(|| Guard(f))
	}
}
impl Drop for Guard {
	fn drop(&mut self) {
		self.0.store(false, Ordering::SeqCst);
	}
}

/// The value of an integer register as the signature's type sees it.
pub fn int_of(ty: Ty, r: u64) -> i64 {
	match ty {
		Ty::I8 => r as i8 as i64,
		Ty::U8 => r as u8 as i64,
		Ty::I16 => r as i16 as i64,
		Ty::U16 => r as u16 as i64,
		Ty::I32 => r as i32 as i64,
		Ty::U32 => r as u32 as i64,
		_ => r as i64,
	}
}

/// The register contents the EE ABI gives an integer of this type: 32-bit values sign-extended to 64 bits.
pub fn canon(ty: Ty, v: i64) -> u64 {
	match ty {
		Ty::U8 => v as u8 as u64,
		Ty::U16 => v as u16 as u64,
		Ty::I32 | Ty::U32 => v as i32 as i64 as u64,
		Ty::I8 => v as i8 as i64 as u64,
		Ty::I16 => v as i16 as i64 as u64,
		_ => v as u64,
	}
}

fn float_of(bits: u32) -> Value {
	Value::float(f32::from_bits(bits) as f64)
}

/// The hook's arguments out of the registers.
pub fn read_args(c: &Cpu, sig: &Sig) -> Vec<Value> {
	let (mut gi, mut fi) = (0usize, 0usize);
	sig.args
		.iter()
		.map(|&ty| {
			if ty.is_float() {
				fi += 1;
				float_of(c.f[11 + fi])
			} else {
				gi += 1;
				Value::int(int_of(ty, c.g(3 + gi)))
			}
		})
		.collect()
}

/// Puts one integer into a register when the typed value there differs (a register that already holds it stays bit for bit).
fn put_int(c: &mut Cpu, reg: usize, ty: Ty, v: i64) {
	if int_of(ty, c.g(reg)) != v {
		c.s(reg, canon(ty, v));
	}
}

fn put_float(c: &mut Cpu, reg: usize, v: f64) {
	if (f32::from_bits(c.f[reg]) as f64).to_bits() != v.to_bits() {
		c.f[reg] = (v as f32).to_bits();
	}
}

/// The (possibly changed) arguments back into the registers, for the retail call.
pub fn put_args(c: &mut Cpu, sig: &Sig, args: &[Value]) {
	let (mut gi, mut fi) = (0usize, 0usize);
	for (&ty, a) in sig.args.iter().zip(args) {
		if ty.is_float() {
			fi += 1;
			put_float(c, 11 + fi, a.f);
		} else {
			gi += 1;
			put_int(c, 3 + gi, ty, a.i);
		}
	}
}

/// What the retail function left in v0 / f0.
pub fn read_ret(c: &Cpu, ty: Ty) -> Value {
	match ty {
		Ty::Void => Value::int(0),
		t if t.is_float() => float_of(c.f[0]),
		t => Value::int(int_of(t, c.g(2))),
	}
}

/// The chain's result into v0 / f0, only where it differs from what is there.
pub fn put_ret(c: &mut Cpu, ty: Ty, v: &Value) {
	match ty {
		Ty::Void => {}
		t if t.is_float() => put_float(c, 0, v.f),
		t => put_int(c, 2, t, v.i),
	}
}

/// The hand of hook N (the binding's hook index): a plain function pointer, as `Hand`/`RelocHand` need.
pub fn hook_n<const N: usize>(c: &mut Cpu) -> bool {
	let s = slot(N);
	// Not the chain's call (slot inactive, native mods stale, nested entry of this hook): the built-in hand that sits on the same
	// entry runs, else retail (layer.rs). The same when the chain never reached the game (`run` false).
	let (active, stale) = (s.active.load(Ordering::Relaxed), super::native_host::stale());
	let taken = if active && !stale { Guard::take(&s.guard) } else { None };
	if super::layer::route(active, stale, taken.is_some()) == super::layer::Route::Under {
		return super::layer::under(N, c);
	}
	let _g = taken;
	run(N, c) || super::layer::under(N, c)
}

fn run(n: usize, c: &mut Cpu) -> bool {
	let s = slot(n);
	let (Some(o), Some(sig)) = (super::get(), s.sig.get()) else { return false };
	let _held = super::lock::enter();
	let (entry, ra) = (c.pc, c.g(31) as u32);
	let sp = super::native_host::sp_before(c);
	let args = read_args(c, sig);
	s.calls.fetch_add(1, Ordering::Relaxed);
	ctx().push(Ctx { slot: n, entry, ra, called: false, unwound: false });
	let (code, ret) = super::host::scope(c, || o.loader.dispatch(n as u32, &args));
	let done = ctx().pop();
	let Some(done) = done else { return false };
	if done.unwound {
		return true;
	}
	// A mod that pushed stack room (stack_push) and never popped it: put the game's stack back.
	super::native_host::sp_check(c, sp, "a hook");
	if code != 0 {
		super::hooks::warn_once(n, &format!("omni_dispatch returned {code}"));
		if !done.called {
			// The chain never reached the game: let the retail code run untouched.
			return false;
		}
	} else {
		put_ret(c, sig.ret, &ret);
	}
	c.pc = ra;
	c.npc = ra.wrapping_add(4);
	c.mem.now = c.icount;
	true
}

/// `call_original` of the host: the retail function with the given arguments, for the dispatch of hook `n` in progress.
pub fn call_retail(c: &mut Cpu, n: usize, sig: &Sig, args: &[Value]) -> Option<Value> {
	let (entry, ra) = {
		let mut st = ctx();
		let cx = st.iter_mut().rev().find(|x| x.slot == n)?;
		if cx.unwound {
			// The machine already left this call at a frame wait: a second `original` has nothing to run on.
			return None;
		}
		cx.called = true;
		(cx.entry, cx.ra)
	};
	put_args(c, sig, args);
	// No frame callback inside the call (frame_wait.rs): a retail function that waits for frames unwinds instead of parking the game
	// thread, with this hook's Luau frame and the loader lock, until the next field.
	// `frame_wait::call` runs the function to its real return (also through a jump out of its compiled span); false only at a frame wait or a stop.
	if !super::frame_wait::call(c, entry, ra) {
		if let Some(cx) = ctx().iter_mut().rev().find(|x| x.slot == n) {
			cx.unwound = true;
		}
		let name = slot(n).id.get().map_or("?", String::as_str).to_string();
		// Where the call stopped: 0x118cb0 = the frame wait (a function that waits for frames); any other pc is a stop. One line per hook and pc.
		let pc = c.pc;
		super::native_host::say_once(
			&format!("unwound-{n}-{pc:x}"),
			&format!("hook {name}: the original function waits for the next frame (stopped at pc {pc:#x}); the rest of it runs without the hook and `original` gave this chain a zero"),
		);
		return None;
	}
	Some(read_ret(c, sig.ret))
}

/// A hook dispatch in progress found its retail call left at a frame wait (`original` returned zero without a result): the machine is
/// parked at the wait for the host loop, so no further game call can be made until the dispatch ends.
pub fn unwound() -> bool {
	unwound_in(&ctx())
}

fn unwound_in(st: &[Ctx]) -> bool {
	st.iter().any(|x| x.unwound)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn cx(slot: usize, unwound: bool) -> Ctx {
		Ctx { slot, entry: 0x100, ra: 0x200, called: false, unwound }
	}

	#[test]
	fn only_the_hooks_on_the_stack_are_dispatching() {
		let st = vec![cx(3, false), cx(7, false)];
		assert!(dispatching_in(&st, 3) && dispatching_in(&st, 7));
		// A chain of hook 3 that calls `original` of hook 5 (omni.fn from inside another hook) is not a dispatch of 5.
		assert!(!dispatching_in(&st, 5));
		assert!(!dispatching_in(&[], 3));
	}

	#[test]
	fn a_call_that_left_at_a_frame_wait_is_seen_anywhere_on_the_stack() {
		assert!(!unwound_in(&[]));
		assert!(!unwound_in(&[cx(1, false), cx(2, false)]));
		assert!(unwound_in(&[cx(1, false), cx(2, true)]));
		assert!(unwound_in(&[cx(1, true), cx(2, false)]));
	}
}
