//! One lock for every call into the OmniLoader DLL (`omni_create`/`omni_load`, `tick`, `pad`, `dispatch`).
//!
//! `init` runs on the main thread, `tick`/`pad` and the hooks on the retail worker thread; they never overlap in time, but the
//! DLL's Luau state is not thread-safe, so exclusion is made by construction here. The lock is re-entrant per thread: a hook
//! runs `omni_dispatch`, the mod calls `original`, the retail function runs and reaches another hooked function, which dispatches
//! again on the same thread.

use std::cell::Cell;
use std::sync::{Mutex, MutexGuard};

static LOCK: Mutex<()> = Mutex::new(());

thread_local! {
	static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Held while this thread is inside the DLL; releases the lock when the outermost one is dropped.
pub struct Held {
	_guard: Option<MutexGuard<'static, ()>>,
}

pub fn enter() -> Held {
	let depth = DEPTH.with(|d| {
		let v = d.get();
		d.set(v + 1);
		v
	});
	// A poisoned lock only means an earlier call panicked; the DLL catches its own panics, so go on.
	let guard = (depth == 0).then(|| LOCK.lock().unwrap_or_else(|e| e.into_inner()));
	Held { _guard: guard }
}

impl Drop for Held {
	fn drop(&mut self) {
		DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn the_lock_is_reentrant_on_one_thread_and_exclusive_across_threads() {
		let a = super::enter();
		let b = super::enter();
		let t = std::thread::spawn(|| {
			let _x = super::enter();
		});
		std::thread::sleep(std::time::Duration::from_millis(30));
		assert!(!t.is_finished(), "another thread must wait while this one is inside");
		drop(b);
		assert!(!t.is_finished());
		drop(a);
		t.join().unwrap();
	}
}
