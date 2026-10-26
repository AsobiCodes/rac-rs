//! The Renderer row (Pause -> Options -> Graphics -> Screen -> Renderer: Vulkan / DirectX 12), retail side: its byte lives in the pcctl
//! block (`pcctl::renderer_dx12`), `sync` hands a change to the options file (`renderer=vulkan|dx12`, pcopts.rs), the window reads that
//! key before it opens (viewer/gpu/renderer.rs), so a change applies after a restart. The values say so on screen: a value that
//! differs from the API running now reads "... - restart" (`value_text`). `note_running` is called by the window's `Gpu::new`.
use super::pcopts::PcOptions;
use std::sync::atomic::{AtomicBool, Ordering};

/// The API the window runs on (set when the window opened).
static RUNNING_DX12: AtomicBool = AtomicBool::new(false);
/// The row as last applied (the file's value until the player changes it).
static WANTED_DX12: AtomicBool = AtomicBool::new(false);

pub fn note_running(dx12: bool) {
	RUNNING_DX12.store(dx12, Ordering::Relaxed);
}

/// `pcext::init`: the file's choice is the row's start value.
pub fn init(opts: &PcOptions) {
	WANTED_DX12.store(opts.renderer_dx12(), Ordering::Relaxed);
}

/// The row's start value for a page built on fresh RAM.
pub fn wanted_dx12() -> bool {
	WANTED_DX12.load(Ordering::Relaxed)
}

/// Worker, every field after the page patch: a changed row goes into the options file (true = the caller saves).
pub fn sync(mem: &mut ee_oracle::mem::Mem, opts: &mut PcOptions) -> bool {
	let Some(dx12) = super::pcctl::renderer_dx12(mem) else { return false };
	if WANTED_DX12.swap(dx12, Ordering::Relaxed) == dx12 {
		return false;
	}
	eprintln!("retail: PC options: Renderer {} (applies after a restart)", if dx12 { "DirectX 12" } else { "Vulkan" });
	opts.renderer = Some(dx12);
	true
}

/// The value text of the row in language `l` (En Fr De Es It); a value that differs from the running API says restart.
pub fn value_text(dx12: bool, l: usize) -> Vec<u8> {
	const RESTART: [&str; 5] = [" - restart", " - relancer", " - Neustart", " - reiniciar", " - riavvio"];
	let mut t = if dx12 { b"DirectX 12".to_vec() } else { b"Vulkan".to_vec() };
	if dx12 != RUNNING_DX12.load(Ordering::Relaxed) {
		t.extend_from_slice(RESTART[l.min(4)].as_bytes());
	}
	t
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_value_that_differs_from_the_running_api_says_restart() {
		note_running(false);
		assert_eq!(value_text(false, 0), b"Vulkan");
		assert_eq!(value_text(true, 0), b"DirectX 12 - restart");
		assert_eq!(value_text(true, 2), b"DirectX 12 - Neustart");
		note_running(true);
		assert_eq!(value_text(true, 0), b"DirectX 12");
		assert_eq!(value_text(false, 3), b"Vulkan - reiniciar");
		note_running(false);
	}
}
