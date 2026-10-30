//! [PC] Background threads with a rayon pool of their own: the level-prefetch thread (scene build) and the vegetation replan run
//! their par_iters there, not on the global frame worker pool (`startup.rs` `init_pool`).
//!
//! Why: the render thread is worker 0 of the global pool (`use_current_thread`), so while it waits inside a `join` / `par_iter`
//! of a frame (`interp::par`, `marker_clip` refresh / selective, `gpuhero::set_poses`, prop skinning) it steals other queued jobs
//! instead of sleeping. With a background `par_iter` on the same pool it could steal a long slice of that and the frame took
//! several ms longer (a 1%-low spike).
//!
//! `spawn` starts a new named thread as before (its thread-locals start fresh, as `retail::defer_*` and the particle `Rng` copy
//! need) and makes it worker 0 of a pool built for that thread alone; the pool's other workers run at below-normal priority
//! (`hot::idle`) and end with the thread. Work started there never reaches the global injector. The results of every parallel
//! collect keep their order, so the output does not depend on which pool ran it. The thread itself runs pieces of its own par_iters
//! too (worker 0): a parallel closure must not use its thread-locals (`retail::defer_*`, the particle `Rng::shared()` copy); none does.

/// Pool size for `frame` frame-pool threads: as many (at least one), so a level prefetch builds as fast as it did on the frame pool;
/// below normal priority, the extra workers only take the cores the frame's threads leave free.
pub(in crate::viewer) fn threads_for(frame: usize) -> usize {
	frame.max(1)
}

/// `std::thread::Builder::new().name(name).stack_size(stack).spawn(f)`, with `f`'s rayon work on a pool of its own (see the module
/// doc). Its workers get the same stack size. Without a pool (the build failed) `f` runs as before, on the global pool.
pub(in crate::viewer) fn spawn<T: Send + 'static>(name: &str, stack: usize, f: impl FnOnce() -> T + Send + 'static) -> std::io::Result<std::thread::JoinHandle<T>> {
	let n = threads_for(rayon::current_num_threads());
	let tag = name.to_string();
	std::thread::Builder::new().name(name.into()).stack_size(stack).spawn(move || {
		let pool = rayon::ThreadPoolBuilder::new()
			.num_threads(n)
			.use_current_thread()
			.stack_size(stack)
			.thread_name(move |i| format!("{tag}-{i}"))
			// Not called for the current thread (index 0): it keeps its priority.
			.start_handler(|_| crate::retail::hot::idle())
			.build();
		match pool {
			Ok(p) => p.install(f),
			Err(_) => f(),
		}
	})
}

#[cfg(test)]
mod tests {
	use rayon::prelude::*;

	#[test]
	fn as_many_threads_at_least_one() {
		assert_eq!([0, 1, 2, 3, 4, 8].map(super::threads_for), [1, 1, 2, 3, 4, 8]);
	}

	/// `f` runs on the spawned thread itself (worker 0 of its own pool), its par_iter keeps its order, and the work ran on that pool.
	#[test]
	fn spawn_runs_on_its_own_thread_and_pool() {
		let h = super::spawn("bgtest", 8 << 20, || {
			let name = std::thread::current().name().map(str::to_string);
			let v = (0..5000u32).into_par_iter().map(|x| x * 2).collect::<Vec<_>>();
			let workers = (0..64u32).into_par_iter().map(|_| std::thread::current().name().unwrap_or("").to_string()).collect::<Vec<_>>();
			(name, rayon::current_thread_index(), v, workers)
		});
		let (name, idx, v, workers) = h.unwrap().join().unwrap();
		assert_eq!(name.as_deref(), Some("bgtest"));
		assert_eq!(idx, Some(0));
		assert!(v.iter().enumerate().all(|(i, &x)| x == i as u32 * 2));
		assert!(workers.iter().all(|w| w == "bgtest" || w.starts_with("bgtest-")), "{workers:?}");
	}
}
