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
use std::any::Any;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// A panic payload carried out of a callback of the DLL (the DLL's frames cannot be unwound through).
pub type Parked = Box<dyn Any + Send>;

/// One dispatch in progress: what `original_cb` needs to run the retail function for it.
pub struct Ctx {
	pub slot: usize,
	pub entry: u32,
	pub ra: u32,
	/// The chain called `original` at least once.
	pub called: bool,
	/// The retail callee did not come back (it waits for a frame, frame_wait.rs): the hand ends at once, the host loop goes on from the wait.
	pub unwound: bool,
	/// The retail code `original` ran panicked (the oracle unwinds the game thread with panics: `RecRebuilt`, `Quit`): the payload waits here
	/// until the dispatch is over, then `run` unwinds with it (`park_panic`). A second `original` of the chain is refused while it waits
	/// (`take_original`): the callee is half-run.
	///
	/// DOUBLE DISPATCH (known, rare): when the unwind is a table rebuild (`RecRebuilt`) that a mod's `call_game` made on the game thread
	/// BEFORE the chain called `original`, the oracle resumes at the pc `Saved::put` restored, which is the hooked function's entry. The
	/// function starts again, the hook dispatches a second time, and a mod that counts or spawns per call runs its chain twice for that one
	/// game call (retail state stays correct). If `original` ran first, the resume pc is the return address and only the mod's return
	/// override is lost. Mod authors: do not rely on exactly-once per call across a `call_game` made before `original`.
	pub panic: Option<Parked>,
}

pub static CTX: Mutex<Vec<Ctx>> = Mutex::new(Vec::new());

fn ctx() -> std::sync::MutexGuard<'static, Vec<Ctx>> {
	CTX.lock().unwrap_or_else(|e| e.into_inner())
}

/// A hook dispatch is in progress (a `call_game` from inside it behaves like `call_original`; between two fields it does not).
pub fn in_hook() -> bool {
	!ctx().is_empty()
}

/// The innermost dispatch in progress is the one of hook `n` (its `original` can take the entry and return address from it). A chain of
/// another hook that calls `original(n)` while the dispatch of `n` is further up the stack does not: that call finds the function by its spec.
pub fn dispatching(n: usize) -> bool {
	dispatching_in(&ctx(), n)
}

fn dispatching_in(st: &[Ctx], n: usize) -> bool {
	st.last().is_some_and(|x| x.slot == n)
}

/// The calling thread is the oracle's game thread (named "ee", oracle `Machine::start`): the only thread whose loop catches `RecRebuilt`.
/// The mods' between-field calls (`tick`, `pad`, events, settings) run on the retail worker thread, with the game thread parked in its
/// vblank callback.
fn on_game_thread() -> bool {
	std::thread::current().name() == Some("ee")
}

/// Something above the calling frame catches an unwind: the oracle's loop on the game thread (`on_game`), or a `run_at` (a mod's
/// `call_game`, `raycast`, ...: `depth` live ones) on the worker between two fields.
fn catcher_above(on_game: bool, depth: u32) -> bool {
	on_game || depth > 0
}

/// A panic of the oracle that no `extern "C"` callback (`original`, `call_game`) can let through.
/// With a catcher above (`catcher_above`) it is kept in the innermost dispatch and `run` unwinds with it once the DLL call returned, a Rust
/// frame under that catcher: on the game thread the oracle's loop, between two fields the `run_at` of the mod's `call_game`. That one
/// swallows it with an empty dispatch stack and `Saved::put` throws the whole half-run call away ("cut short"), so the registers and stack
/// a half-run callee left never reach the caller (the compiled frames above, of the old table, run on only with the registers `Saved::put`
/// restored). With no catcher (a hook reached from non-omni retail code on the worker, no `run_at` live) it is swallowed in place; either
/// way the table rebuild it announces is passed on to the game thread through `rec_unwind`, the flag the vblank callback sets after its own
/// rebuild: the game thread unwinds its stale compiled frames when it resumes.
pub fn park_panic(c: &mut Cpu, p: Parked) {
	let mut unwind = false;
	park_in(&mut ctx(), p, catcher_above(on_game_thread(), super::native_host::calls_live()), &mut unwind);
	if unwind {
		c.rec_unwind = true;
	}
}

fn park_in(st: &mut [Ctx], p: Parked, catcher: bool, unwind: &mut bool) {
	match st.last_mut() {
		// The first one wins: a later panic is a consequence of the machine already being cut short.
		Some(cx) if catcher => {
			cx.panic.get_or_insert(p);
		}
		_ => *unwind |= is_unwind_signal(&p),
	}
}

/// A panic payload that is no text: the oracle's way to unwind the game thread (`RecRebuilt`, `Quit`; a `panic!` always carries a text).
pub fn is_unwind_signal(p: &Parked) -> bool {
	!(p.is::<&str>() || p.is::<String>())
}

/// A panic is waiting to unwind the game thread: no further game call may run on the machine until it has.
pub fn panic_parked() -> bool {
	ctx().iter().any(|x| x.panic.is_some())
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
	ctx().push(Ctx { slot: n, entry, ra, called: false, unwound: false, panic: None });
	let (code, ret) = super::host::scope(c, || o.loader.dispatch(n as u32, &args));
	let done = ctx().pop();
	let Some(done) = done else { return false };
	if let Some(p) = done.panic {
		// The retail code under `original` unwound with a panic (the oracle's rebuild / quit, or a real one): passed on to the game thread's
		// loop from here, a Rust frame, as it would unwind without the hook.
		std::panic::resume_unwind(p);
	}
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

/// The entry and return address `original` of hook `n` calls, taken from the innermost dispatch. None: that is no dispatch of `n`, or the
/// machine already left this call at a frame wait or a panic is parked in it (the retail code under an earlier `original` was cut short,
/// half-run): a second `original` has nothing to run on and must not start the callee again on that state.
fn take_original(st: &mut [Ctx], n: usize) -> Option<(u32, u32)> {
	let cx = st.last_mut().filter(|x| x.slot == n)?;
	if cx.unwound || cx.panic.is_some() {
		return None;
	}
	cx.called = true;
	Some((cx.entry, cx.ra))
}

/// `call_original` of the host: the retail function with the given arguments, for the dispatch of hook `n` in progress.
pub fn call_retail(c: &mut Cpu, n: usize, sig: &Sig, args: &[Value]) -> Option<Value> {
	let (entry, ra) = take_original(&mut ctx(), n)?;
	put_args(c, sig, args);
	// No frame callback inside the call (frame_wait.rs): a retail function that waits for frames unwinds instead of parking the game
	// thread, with this hook's Luau frame and the loader lock, until the next field.
	// `frame_wait::call` runs the function to its real return (also through a jump out of its compiled span); false only at a frame wait or a stop.
	if !super::frame_wait::call(c, entry, ra) {
		if let Some(cx) = ctx().last_mut().filter(|x| x.slot == n) {
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
		Ctx { slot, entry: 0x100, ra: 0x200, called: false, unwound, panic: None }
	}

	#[test]
	fn only_the_innermost_dispatch_takes_its_entry() {
		let st = vec![cx(3, false), cx(7, false)];
		assert!(dispatching_in(&st, 7));
		// Hook 7's chain calls `original` of hook 3, whose dispatch is further up the stack: that is no dispatch of 3 (F2 of the review).
		assert!(!dispatching_in(&st, 3));
		// Nor one of a hook (5) that is not on the stack at all.
		assert!(!dispatching_in(&st, 5));
		assert!(!dispatching_in(&[], 3));
	}

	#[test]
	fn only_a_panic_without_text_is_an_unwind_signal() {
		struct Rebuilt;
		assert!(is_unwind_signal(&(Box::new(Rebuilt) as Parked)));
		assert!(!is_unwind_signal(&(Box::new("boom") as Parked)));
		assert!(!is_unwind_signal(&(Box::new(String::from("boom")) as Parked)));
	}

	#[test]
	fn a_panic_parked_on_the_game_thread_waits_in_the_innermost_dispatch() {
		struct Rebuilt;
		let mut st = vec![cx(3, false), cx(7, false)];
		let mut unwind = false;
		park_in(&mut st, Box::new(Rebuilt), true, &mut unwind);
		assert!(st[0].panic.is_none() && st[1].panic.as_ref().is_some_and(|p| p.is::<Rebuilt>()) && !unwind);
		// The first panic stays.
		park_in(&mut st, Box::new("later"), true, &mut unwind);
		assert!(st[1].panic.as_ref().is_some_and(|p| p.is::<Rebuilt>()));
	}

	#[test]
	fn between_fields_an_unwind_signal_is_swallowed_and_passed_on_as_a_flag() {
		struct Rebuilt;
		// Worker thread, with and without a dispatch on the stack: nothing is parked (nothing there would catch it), the flag is set.
		for mut st in [vec![], vec![cx(3, false)]] {
			let mut unwind = false;
			park_in(&mut st, Box::new(Rebuilt), false, &mut unwind);
			assert!(unwind && st.iter().all(|x| x.panic.is_none()));
		}
		// A text panic is no rebuild: swallowed without the flag.
		let mut unwind = false;
		park_in(&mut [], Box::new("boom"), false, &mut unwind);
		assert!(!unwind);
	}

	#[test]
	fn a_catcher_is_the_game_thread_or_a_live_game_call() {
		// F1 of review 3: a worker dispatch under a mod's `call_game` has the `run_at` above it, so it parks like the game thread does.
		assert!(catcher_above(true, 0));
		assert!(catcher_above(false, 1));
		assert!(catcher_above(false, 3));
		// Nothing above (a hook reached from plain retail code between two fields): swallowed in place.
		assert!(!catcher_above(false, 0));
	}

	#[test]
	fn a_worker_dispatch_under_a_game_call_parks_the_panic_and_refuses_a_second_original() {
		struct Rebuilt;
		let mut st = vec![cx(3, false)];
		// The decision `park_panic` takes on the worker with one `run_at` live (depth 1).
		let mut unwind = false;
		park_in(&mut st, Box::new(Rebuilt), catcher_above(false, 1), &mut unwind);
		assert!(st[0].panic.is_some() && !unwind, "parked for `run` to re-throw to the run_at, not swallowed in place");
		// The half-run callee is not started again by a second `original` of the same chain.
		assert!(take_original(&mut st, 3).is_none());
		assert!(!st[0].called);
		// Without the park (a fresh dispatch) the first `original` takes the entry and return address.
		let mut fresh = vec![cx(3, false)];
		assert_eq!(take_original(&mut fresh, 3), Some((0x100, 0x200)));
		assert!(fresh[0].called);
		// Another hook's `original` finds no dispatch of its own.
		assert!(take_original(&mut fresh, 4).is_none());
	}

	#[test]
	fn park_panic_between_fields_sets_rec_unwind_and_throws_nothing() {
		struct Rebuilt;
		// This test thread is no "ee" thread: the between-field path. The Cpu is a real one; no unwind may escape and none may stay parked.
		let mut c = Cpu::new(ee_oracle::mem::Mem::new());
		assert!(!c.rec_unwind);
		let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| park_panic(&mut c, Box::new(Rebuilt))));
		assert!(r.is_ok(), "park_panic itself must not unwind");
		assert!(c.rec_unwind, "the game thread must unwind its stale frames when it resumes");
		assert!(!panic_parked());
	}

	#[test]
	fn a_panic_parked_on_a_thread_named_ee_stays_with_its_dispatch() {
		struct Rebuilt;
		let h = std::thread::Builder::new()
			.name("ee".into())
			.spawn(|| {
				assert!(on_game_thread());
				let mut st = vec![cx(1, false)];
				let mut unwind = false;
				park_in(&mut st, Box::new(Rebuilt), on_game_thread(), &mut unwind);
				!unwind && st[0].panic.is_some()
			})
			.unwrap();
		assert!(h.join().unwrap());
		assert!(!on_game_thread());
	}

	#[test]
	fn a_call_that_left_at_a_frame_wait_is_seen_anywhere_on_the_stack() {
		assert!(!unwound_in(&[]));
		assert!(!unwound_in(&[cx(1, false), cx(2, false)]));
		assert!(unwound_in(&[cx(1, false), cx(2, true)]));
		assert!(unwound_in(&[cx(1, true), cx(2, false)]));
	}
}
