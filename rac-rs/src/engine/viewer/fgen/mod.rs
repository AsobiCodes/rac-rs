//! [PC] Frame generation (lane fgen; plan scratch_perf_fgen/FRAMEGEN_PLAN.md). Stage S1: the present takeover, no generation yet.
//!
//! wgpu keeps owning the window in the normal path (`present::Present::Wgpu`: today's code, cost and device). `RAC_FG=null` puts
//! our own swapchain on the window instead (`present::Present::Own`): every pass draws into our own window-size backbuffer (the
//! frame's `FrameImage::Own`), the frame's last encoder copies it onto the acquired swapchain image and moves that image to the
//! present state, and the present thread presents it (Vulkan: plain ash swapchain through the `swap_vk::SwapFns` table the vendor
//! swapchains replace later; DirectX 12: a plain DXGI flip swapchain made for the window's HWND on wgpu's command queue). The
//! picture is the same; only who owns the window's swapchain changes.
//!
//! The extra-queue device plan (`queues.rs`): when a frame-generation DLL sits next to the exe (or RAC_FG_QUEUES=1), the Vulkan
//! device asks for the extra queues the vendors want; RAC_FG=0 = always today's device.
//!
//! Stage S2 (`RAC_FG=fsr`, ffx_fg.rs): the own swapchain is FidelityFX's frame-generation swapchain (Vulkan: its replacement
//! swapchain functions in `swap_vk::SwapFns`; DirectX 12: its proxy IDXGISwapChain4). Per frame frame.rs `fg_encode` records the
//! prepare dispatch (depth + motion vectors, Vulkan) or hands it to the present thread (DirectX 12), and copies the picture
//! before the HUD (the hudless input); the present thread configures the context with that frame id right before the present.

pub mod choice;
#[cfg(windows)]
mod ffx_fg;
#[cfg(windows)]
mod ffx_fg_abi;
mod frame;
pub mod follow;
pub(crate) mod present;
pub(crate) mod queues;
#[cfg(windows)]
pub(crate) mod sl;
#[cfg(windows)]
mod sl_abi;
#[cfg(windows)]
pub(crate) mod sl_card;
#[cfg(windows)]
mod sl_frame;
#[cfg(windows)]
pub(crate) mod sl_vk;
#[cfg(windows)]
mod swap_dx12;
#[cfg(windows)]
mod swap_vk;
#[cfg(test)]
mod tests;

/// What `RAC_FG` asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Mode {
	/// Unset: wgpu's surface; the extra queues only when a frame-generation DLL is next to the exe.
	Auto,
	/// `RAC_FG=0` / `off`: wgpu's surface and exactly today's device.
	Off,
	/// `RAC_FG=null`: our own swapchain, no generation (stage S1's proof path).
	Null,
	/// `RAC_FG=fsr`: our own swapchain made by AMD's FidelityFX frame generation (stage S2): one generated frame between two
	/// rendered ones. A missing DLL or a card it refuses: wgpu's surface, one log line.
	Fsr,
	/// `RAC_FG=dlss2|dlss3|dlss4` (S3, sl.rs): NVIDIA DLSS frame generation through Streamline on our own swapchain, the number of
	/// generated frames per rendered one (1..3 = 2x..4x). DirectX 12 only in this build; refused (one log line) elsewhere.
	Dlss(u32),
}

/// The mode of an `RAC_FG` value (pure).
pub(crate) fn mode_of(v: Option<&str>) -> Mode {
	match v.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
		Some("0" | "off") => Mode::Off,
		Some("null") => Mode::Null,
		Some("fsr") => Mode::Fsr,
		Some("dlss2") => Mode::Dlss(1),
		Some("dlss3") => Mode::Dlss(2),
		Some("dlss4") => Mode::Dlss(3),
		_ => Mode::Auto,
	}
}

/// [S2 debug] `RAC_FG_TRACE=1`: the first 300 frame-generation steps as stderr lines (where a frame stops, which call crashed).
#[allow(dead_code)]
pub(crate) fn trace(what: std::fmt::Arguments<'_>) {
	use std::sync::atomic::{AtomicU32, Ordering};
	static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	static N: AtomicU32 = AtomicU32::new(0);
	if *ON.get_or_init(|| std::env::var("RAC_FG_TRACE").is_ok_and(|v| v == "1")) && N.fetch_add(1, Ordering::Relaxed) < 300 {
		eprintln!("fgen trace: {what}");
	}
}

/// The mode of an `RAC_FG` value and the options file's `framegen` value (pure, S4): a set, non-empty env wins (tests), else the
/// file's choice (the menu row, `choice.rs`), else Auto.
pub(crate) fn mode_with(env: Option<&str>, file: Option<&str>) -> Mode {
	match env.filter(|v| !v.trim().is_empty()) {
		Some(v) => mode_of(Some(v)),
		// Only a generating value counts: `framegen=off` (or junk) keeps today's Auto, never the forced-plain-device Off.
		None if choice::index(file) != 0 => mode_of(file),
		None => Mode::Auto,
	}
}

/// `RAC_FG`, else the options file's `framegen` (the Frame Generation row), read once before the window's device is made.
pub(crate) fn mode() -> Mode {
	static M: std::sync::OnceLock<Mode> = std::sync::OnceLock::new();
	*M.get_or_init(|| {
		let env = std::env::var("RAC_FG").ok();
		let m = mode_with(env.as_deref(), if env.as_deref().is_some_and(|v| !v.trim().is_empty()) { None } else { choice::file_value() });
		if matches!(m, Mode::Fsr | Mode::Dlss(_)) && env.is_none() {
			eprintln!("fgen: Frame Generation {m:?} from the options file (framegen)");
		}
		m
	})
}

/// [Test] `RAC_FG_FAILTEST=1`: FidelityFX's swapchain create fails on purpose (Vulkan and DirectX 12), to prove the fallback to
/// wgpu's surface (present.rs `Screen::fall_back`). Unset: Ok.
#[allow(dead_code)]
pub(crate) fn failtest(api: &str) -> Result<(), String> {
	static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	if *ON.get_or_init(|| std::env::var("RAC_FG_FAILTEST").is_ok_and(|v| v == "1")) {
		return Err(format!("RAC_FG_FAILTEST=1: the {api} frame-generation swapchain refused on purpose"));
	}
	Ok(())
}
