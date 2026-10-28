//! Turning retail mode on for the next game start (`enable`, `enable_cold`, `--gsfull`) and asking whether it is on.

use super::*;

pub(super) static STATE: OnceLock<String> = OnceLock::new();

/// The savestate's level byte (0x15ed84) read by `enable`: the level `view` loads.
static START_LEVEL: OnceLock<u32> = OnceLock::new();

/// GS-full display mode requested (`--gsfull`; RAC_GSFULL=1 does the same): the game's own picture from the GS.
static GSFULL: AtomicBool = AtomicBool::new(false);

/// `--retail-cold` (the ISO path): no savestate, `Machine::cold` boots the retail game from the disc (logos, title, New Game, Veldin).
pub(super) static COLD: OnceLock<String> = OnceLock::new();

/// Turns retail mode on for the next game start; the prop pools come from the state's moby table.
pub fn enable(dir: &str) -> Result<(), String> {
	let ram = std::fs::read(format!("{dir}/eeMemory.bin")).map_err(|e| format!("{dir}/eeMemory.bin: {e}"))?;
	// RAC_RETAIL_REC / RAC_RETAIL_REPLAY: opened here so a bad path stops the start instead of running unrecorded.
	rec::open_from_env()?;
	rec::replay_from_env()?;
	let at = |a: u32| ram.get(a as usize..a as usize + 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()));
	let level = ram[LEVEL as usize] as u32;
	set_pools(count_pools(level, at, |a| at(a) as u16));
	// The disc of the run (ORACLE_ISO, else the oracle's default PAL image) and its video mode (retail/disc.rs).
	ee_oracle::ver::from_iso(&std::env::var("ORACLE_ISO").unwrap_or_else(|_| ee_oracle::DEFAULT_ISO.into()));
	// Every recompiled-table build (a level load) voids the hook layer's cache and the code guard's spans, as on the cold boot.
	ee_oracle::ver::on_rebuild(disc::rebuilt);
	let _ = STATE.set(dir.to_string());
	let _ = START_LEVEL.set(level);
	Ok(())
}

/// Turns retail mode on for the next game start without a savestate (`--retail-cold`): the machine is the cold boot of the retail
/// ELF from `iso` (ee_oracle `Machine::cold`); the first level is Veldin (0), loaded after the front end's New Game.
pub fn enable_cold(iso: &str) -> Result<(), String> {
	rec::open_from_env()?;
	rec::replay_from_env()?;
	// The oracle reads level data (overlay signatures, sound banks) from the same disc: ORACLE_ISO, else its default path.
	if std::env::var_os("ORACLE_ISO").is_none() {
		// SAFETY: called from main before any other thread exists.
		unsafe { std::env::set_var("ORACLE_ISO", iso) };
	}
	let _ = COLD.set(iso.to_string());
	// The disc (PAL or USA) and its video mode (retail/disc.rs).
	ee_oracle::ver::from_iso(iso);
	disc::announce();
	ee_oracle::ver::on_rebuild(disc::rebuilt);
	let _ = STATE.set("cold:".to_string());
	let _ = START_LEVEL.set(0);
	Ok(())
}

/// `rac-rs warm-gpu` (setup, launchers): opens the GPU capture once so the driver builds and keeps its kernels; a first
/// start of the game then has them at once instead of drawing on the CPU until they are built.
pub fn warm_gpu() -> Result<(), String> {
	if gscap::gpu_warm() { Ok(()) } else { Err("no GPU for the capture".into()) }
}

/// The window's device (viewer/gpu/setup.rs, before the game loads): the GS capture draws on it, except on an NVIDIA Vulkan window,
/// where it gets a second device (own queue) on the window's adapter; DX12 windows always share (gscap draw/gpu_shared.rs).
pub fn gpu_capture_window(dev: &wgpu::Device, q: &wgpu::Queue, info: &wgpu::AdapterInfo, adapter: &wgpu::Adapter) {
	gscap::gpu_window(dev, q, info, adapter);
}

/// The window is about to open its device (viewer/handler.rs `resumed`): the GS capture waits for it.
pub fn gpu_capture_expect_window() {
	gscap::gpu_expect_window();
}

/// Features the window's device asks for so the GS capture can share it (setup.rs `Gpu::new`).
pub fn gpu_capture_features(adapter: &wgpu::Adapter) -> wgpu::Features {
	gscap::gpu_window_features(adapter)
}

/// Held around `Surface::configure` on the window's device: the shared GS capture must not submit meanwhile (gscap draw/gpu_shared.rs).
pub fn gpu_capture_submit_lock() -> std::sync::MutexGuard<'static, ()> {
	gscap::gpu_submit_lock()
}

pub fn active() -> bool {
	STATE.get().is_some()
}

/// Turns the GS-full display on for the next game start (`--gsfull`).
pub fn enable_gsfull() {
	GSFULL.store(true, Ordering::SeqCst);
}

/// Whether retail mode shows the game's own GS picture (`--gsfull` or RAC_GSFULL=1; default off).
pub fn gsfull_on() -> bool {
	GSFULL.load(Ordering::SeqCst) || std::env::var("RAC_GSFULL").is_ok_and(|v| v != "0" && !v.is_empty())
}

/// The level of the savestate `enable` loaded (None outside retail mode).
pub fn start_level() -> Option<u32> {
	START_LEVEL.get().copied()
}

#[cfg(test)]
mod tests {
	/// A savestate start (`enable`) and the cold boot (`enable_cold`) both register the table-rebuild hook (`ver::on_rebuild`), which voids the hook
	/// layer's cache and the mod code guard's spans (review N2). Both functions need a disc and a process-wide state to run, so the check reads
	/// the source: each function body must name the registration.
	#[test]
	fn both_game_starts_register_the_rebuild_hook() {
		let src = include_str!("session.rs");
		let body = |from: &str, to: &str| {
			let a = src.find(from).unwrap_or_else(|| panic!("{from} not found"));
			let b = a + src[a..].find(to).unwrap_or_else(|| panic!("{to} not found"));
			&src[a..b]
		};
		let enable = body("pub fn enable(dir", "pub fn enable_cold(");
		let cold = body("pub fn enable_cold(", "pub fn warm_gpu(");
		for (name, text) in [("enable", enable), ("enable_cold", cold)] {
			assert!(text.contains("ee_oracle::ver::on_rebuild(disc::rebuilt)"), "{name} does not register disc::rebuilt with ver::on_rebuild");
		}
	}
}