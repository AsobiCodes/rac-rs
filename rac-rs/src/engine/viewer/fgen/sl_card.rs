//! [S3] Which graphics card DLSS frame generation would run on, asked of DXGI without making a device (lane fgdl review fixes 1,
//! 2, 6): the adapters in the high-performance order wgpu's pick follows (renderer.rs `HighPerformance`), software adapters left
//! out, each with its PCI vendor and its user-mode driver version. Used before the Vulkan instance is made (sl_vk.rs `wanted`:
//! only an NVIDIA card goes through Streamline), by the menus (choice.rs `offer_dlss`: no NVIDIA card = no DLSS values) and for the
//! card memory (choice.rs: the remembered card is name + driver; a new driver forgets a refusal).

use super::choice::CardMemory;
use std::sync::OnceLock;

/// PCI vendor id of NVIDIA.
pub(crate) const NVIDIA: u32 = 0x10DE;

/// One DXGI adapter.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DxCard {
	pub name: String,
	pub vendor: u32,
	/// The user-mode driver version ("32.0.15.8157"; empty when DXGI does not say).
	pub driver: String,
}

/// The driver version of IDXGIAdapter::CheckInterfaceSupport's LARGE_INTEGER as Windows shows it (pure).
pub(crate) fn driver_string(umd: i64) -> String {
	let v = umd as u64;
	format!("{}.{}.{}.{}", v >> 48, (v >> 32) & 0xffff, (v >> 16) & 0xffff, v & 0xffff)
}

/// Whether the window's Vulkan instance may be made through Streamline for `card` (the adapter the game will use) given the
/// remembered card (pure): only an NVIDIA card, and not one remembered with this driver as running no DLSS frame generation. A
/// memory of another card or driver cannot be verified here and is ignored (choice.rs `card_seen` forgets it at the device).
pub(crate) fn vk_gate(card: Option<&DxCard>, memory: Option<&CardMemory>) -> Result<(), String> {
	let Some(c) = card else { return Err("no DirectX adapter found to check the card".into()) };
	if c.vendor != NVIDIA {
		return Err(format!("{} is not an NVIDIA card", c.name));
	}
	if memory.is_some_and(|m| m.max == 0 && m.is(&c.name, &c.driver)) {
		return Err("this card was refused before with this driver (framegen_card.txt)".into());
	}
	Ok(())
}

/// The adapters, high-performance first (once per run; empty when DXGI fails).
pub(crate) fn list() -> &'static [DxCard] {
	static L: OnceLock<Vec<DxCard>> = OnceLock::new();
	L.get_or_init(|| enumerate().unwrap_or_else(|e| {
		eprintln!("fgen: DXGI adapters: {e}");
		Vec::new()
	}))
}

/// The adapter the game will most likely use (wgpu's high-performance pick).
pub(crate) fn preferred() -> Option<&'static DxCard> {
	list().first()
}

/// Whether any adapter of this PC is an NVIDIA card.
pub(crate) fn nvidia_present() -> bool {
	list().iter().any(|c| c.vendor == NVIDIA)
}

/// The driver version of the adapter named `name` (the window's adapter by wgpu's name for it).
pub(crate) fn driver_of(name: &str) -> Option<&'static str> {
	list().iter().find(|c| c.name == name).map(|c| c.driver.as_str())
}

fn enumerate() -> Result<Vec<DxCard>, String> {
	use windows::Win32::Graphics::Dxgi::*;
	use windows::core::Interface;
	// SAFETY: plain DXGI calls on objects we own; nothing is kept past this function.
	let f: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.map_err(|e| format!("CreateDXGIFactory1 {e}"))?;
	let f6: Option<IDXGIFactory6> = f.cast().ok();
	let mut out = Vec::new();
	for i in 0..16u32 {
		let a: IDXGIAdapter1 = match &f6 {
			Some(f6) => match unsafe { f6.EnumAdapterByGpuPreference(i, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE) } {
				Ok(a) => a,
				Err(_) => break,
			},
			None => match unsafe { f.EnumAdapters1(i) } {
				Ok(a) => a,
				Err(_) => break,
			},
		};
		let Ok(d) = (unsafe { a.GetDesc1() }) else { continue };
		if d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
			continue;
		}
		let n = d.Description.iter().position(|&c| c == 0).unwrap_or(d.Description.len());
		let driver = unsafe { a.CheckInterfaceSupport(&IDXGIDevice::IID) }.map(driver_string).unwrap_or_default();
		out.push(DxCard { name: String::from_utf16_lossy(&d.Description[..n]), vendor: d.VendorId, driver });
	}
	Ok(out)
}
