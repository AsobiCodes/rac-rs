//! [PC] The run log's list of graphics adapters when the window found none (GitHub #49). The log used to say only "no Vulkan adapter on
//! this PC"; a bug report then could not tell a missing driver from an adapter that wgpu listed but that cannot present to the window,
//! or one that wgpu hides. Log only: nothing here changes which adapter is used. `renderer.rs` calls `report` when `request_adapter` fails.

/// One adapter as one log line: backend, name, vendor and device ids, type, driver and whether it can present to the window.
pub(super) fn line(i: &wgpu::AdapterInfo, can_present: bool) -> String {
	format!(
		"gpu:   adapter {:?} \"{}\", vendor {:#06x} device {:#06x}, {:?}, driver \"{}\" \"{}\", can present to the window: {}",
		i.backend,
		i.name,
		i.vendor,
		i.device,
		i.device_type,
		i.driver,
		i.driver_info,
		if can_present { "yes" } else { "no" }
	)
}

/// The note after the list, when it can explain an empty one: wgpu hides a Vulkan adapter whose driver reports no conformance version
/// (wgpu-hal `vulkan/adapter.rs`) unless `ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER` is set.
pub(super) fn hidden_note(backends: wgpu::Backends, flags: wgpu::InstanceFlags) -> Option<&'static str> {
	(backends.contains(wgpu::Backends::VULKAN) && !flags.contains(wgpu::InstanceFlags::ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER)).then_some(
		"gpu:   note: wgpu hides a Vulkan adapter whose driver reports no conformance version; WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER=1 lists it",
	)
}

/// Logs every adapter of `backends` that `instance` lists and whether it can present to `surface` (nothing listed: says so), then the
/// note about hidden adapters.
pub(super) fn report(instance: &wgpu::Instance, surface: &wgpu::Surface<'static>, backends: wgpu::Backends, flags: wgpu::InstanceFlags) {
	let adapters = pollster::block_on(instance.enumerate_adapters(backends));
	eprintln!("gpu: {backends:?}: {} adapter(s) listed", adapters.len());
	for a in &adapters {
		eprintln!("{}", line(&a.get_info(), a.is_surface_supported(surface)));
	}
	if let Some(note) = hidden_note(backends, flags) {
		eprintln!("{note}");
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn info() -> wgpu::AdapterInfo {
		let mut i = wgpu::AdapterInfo::new(wgpu::DeviceType::IntegratedGpu, wgpu::Backend::Vulkan);
		(i.name, i.vendor, i.device) = ("PS5".into(), 0x1002, 0);
		(i.driver, i.driver_info) = ("radv".into(), "Mesa 26.2.0".into());
		i
	}

	#[test]
	fn an_adapter_line_names_everything_a_report_needs() {
		let l = line(&info(), false);
		assert_eq!(l, "gpu:   adapter Vulkan \"PS5\", vendor 0x1002 device 0x0000, IntegratedGpu, driver \"radv\" \"Mesa 26.2.0\", can present to the window: no");
		assert!(line(&info(), true).ends_with("can present to the window: yes"));
	}

	#[test]
	fn the_hidden_adapter_note_only_where_it_can_apply() {
		let none = wgpu::InstanceFlags::empty();
		let allow = wgpu::InstanceFlags::ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER;
		assert!(hidden_note(wgpu::Backends::VULKAN, none).is_some());
		assert!(hidden_note(wgpu::Backends::VULKAN | wgpu::Backends::DX12, none).is_some());
		assert!(hidden_note(wgpu::Backends::VULKAN, allow).is_none(), "already allowed: nothing is hidden");
		assert!(hidden_note(wgpu::Backends::DX12, none).is_none(), "DirectX 12 has no such filter");
	}
}
