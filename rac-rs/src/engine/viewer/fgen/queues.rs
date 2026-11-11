//! [PC] Frame generation (lane fgen, S1): the extra-queue device plan. FidelityFX frame generation on Vulkan wants an async-compute,
//! a present and an image-acquire queue besides the game queue. wgpu opens one queue; only when the chosen mode is FSR (RAC_FG=fsr
//! or the options file's framegen=fsr) or the RAC_FG_QUEUES=1 test switch asks, the Vulkan device is opened through wgpu-hal's
//! `open_with_callback` with these queues added (`add`; gpu/dlss_vk.rs, also inside NGX's callback). Every other run (framegen
//! off, RAC_FG unset) opens exactly today's device, DLLs next to the exe or not. FidelityFX never gets the game's own VkQueue
//! (queue 0 of the game family): wgpu and the presenter submit there, FidelityFX's present thread would race them. No such queue
//! (one graphics queue and no other present-capable family): no plan, FSR frame generation is refused on Vulkan.

use super::Mode;

/// One queue family as the plan sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Family {
	pub graphics: bool,
	pub compute: bool,
	/// The family can present to a window (Windows: vkGetPhysicalDeviceWin32PresentationSupportKHR).
	pub present: bool,
	/// The family's queue count.
	pub count: u32,
}

/// Where each extra queue lands: (family, queue index in that family).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) struct Roles {
	pub compute: (u32, u32),
	pub present: (u32, u32),
	pub acquire: (u32, u32),
}

/// The plan: per family how many queues to open in all (the game queue's family included), and the roles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Plan {
	pub counts: Vec<(u32, u32)>,
	pub roles: Roles,
}

/// The plan for a device whose game queue is queue 0 of family `main` (pure). Async compute: a compute-only family when there is
/// one, else the next queue of `main`. Present: the next queue of `main` when it has a second one and presents, else a
/// present-capable other family (the compute-only one first; AMD and RADV compute queues present). Image acquire: like async
/// compute. A family with no room left shares its last opened queue between roles, but no role ever gets the game queue
/// (`main`, 0): None when that is the only choice.
pub(crate) fn plan(families: &[Family], main: u32) -> Option<Plan> {
	let m = families.get(main as usize)?;
	if !m.graphics || m.count == 0 {
		return None;
	}
	let cfam = families.iter().position(|f| f.compute && !f.graphics && f.count > 0).map(|i| i as u32);
	let other_present = cfam.filter(|&c| families[c as usize].present).or_else(|| families.iter().enumerate().position(|(i, f)| i as u32 != main && f.present && f.count > 0).map(|i| i as u32));
	let mut counts: Vec<(u32, u32)> = vec![(main, 1)];
	let mut take = |fam: u32| -> Option<(u32, u32)> {
		let cap = families[fam as usize].count;
		let i = match counts.iter().position(|e| e.0 == fam) {
			Some(i) => i,
			None => {
				counts.push((fam, 0));
				counts.len() - 1
			}
		};
		if counts[i].1 < cap {
			counts[i].1 += 1;
		}
		let q = (fam, counts[i].1.max(1) - 1);
		(q != (main, 0)).then_some(q)
	};
	let compute = take(cfam.unwrap_or(main))?;
	let present = if m.count >= 2 && m.present { take(main)? } else { take(other_present?)? };
	let acquire = take(cfam.unwrap_or(main))?;
	counts.retain(|e| e.1 > 0);
	Some(Plan { counts, roles: Roles { compute, present, acquire } })
}

/// Whether the device asks for the extra queues (pure): with the FSR mode (RAC_FG=fsr or framegen=fsr), or the RAC_FG_QUEUES=1
/// test switch unless RAC_FG=0. Never otherwise: framegen off / RAC_FG unset = exactly today's device.
pub(crate) fn wanted(mode: Mode, env_forced: bool) -> bool {
	mode == Mode::Fsr || mode != Mode::Off && env_forced
}

/// `wanted` for this run (RAC_FG / the options file, RAC_FG_QUEUES; Windows only: FidelityFX runs nowhere else), with its log line.
pub(crate) fn wanted_now() -> bool {
	let forced = std::env::var("RAC_FG_QUEUES").is_ok_and(|v| v == "1");
	let mode = super::mode();
	let on = cfg!(windows) && wanted(mode, forced);
	if on {
		eprintln!("fgen: frame-generation queues wanted ({})", if mode == Mode::Fsr { "Frame Generation FSR" } else { "RAC_FG_QUEUES=1" });
	} else {
		eprintln!("fgen: today's device (no frame-generation queues)");
	}
	on
}

/// The roles of the queues the window's device opened (None: today's device).
pub(crate) fn made() -> Option<Roles> {
	MADE.get().copied().flatten()
}

static MADE: std::sync::OnceLock<Option<Roles>> = std::sync::OnceLock::new();
/// The plan `add` put into the last device create; `commit` makes it the device's once that device is the one in use.
static PENDING: std::sync::Mutex<Option<Plan>> = std::sync::Mutex::new(None);

/// After the device with the plan's queues was made (create_device_from_hal Ok): its roles count from here; logged once.
pub(crate) fn commit() {
	let Some(p) = PENDING.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
	let r = p.roles;
	eprintln!("fgen: Vulkan device with frame-generation queues {:?} (family, count); queue roles: game ({}, 0), async compute {:?}, present {:?}, image acquire {:?}", p.counts, p.counts[0].0, r.compute, r.present, r.acquire);
	let _ = MADE.set(Some(r));
}
/// The queue families of a Vulkan adapter, as the plan sees them.
#[cfg(not(target_vendor = "apple"))]
pub(crate) fn families(adapter: &wgpu::Adapter) -> Option<Vec<Family>> {
	use ash::vk::QueueFlags;
	let a = unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }?;
	let (inst, pd) = (a.shared_instance(), a.raw_physical_device());
	let props = unsafe { inst.raw_instance().get_physical_device_queue_family_properties(pd) };
	#[cfg(windows)]
	let w32 = ash::khr::win32_surface::Instance::new(inst.entry(), inst.raw_instance());
	let present = |i: usize| -> bool {
		#[cfg(windows)]
		return unsafe { w32.get_physical_device_win32_presentation_support(pd, i as u32) };
		#[cfg(not(windows))]
		{
			let _ = i;
			false
		}
	};
	Some(props.iter().enumerate().map(|(i, p)| Family { graphics: p.queue_flags.contains(QueueFlags::GRAPHICS), compute: p.queue_flags.contains(QueueFlags::COMPUTE), present: present(i), count: p.queue_count }).collect())
}

/// Inside a wgpu-hal device callback: adds the plan's queues to the queue create infos (wgpu's own entry is the game queue's family,
/// queue 0) and keeps the plan pending (`commit` once the device is made). The priorities live for the program (leaked, a few
/// bytes once per device). No plan (no queue besides the game queue): the infos stay as they are, one log line.
#[cfg(not(target_vendor = "apple"))]
pub(crate) fn add(infos: &mut Vec<ash::vk::DeviceQueueCreateInfo<'_>>, families: &[Family]) {
	let Some(main) = infos.first().map(|i| i.queue_family_index) else { return };
	let Some(p) = plan(families, main) else {
		eprintln!("fgen: no Vulkan queue for FidelityFX besides the game queue ({families:?}): today's device, no FSR frame generation on Vulkan");
		*PENDING.lock().unwrap_or_else(|e| e.into_inner()) = None;
		return;
	};
	for &(fam, n) in &p.counts {
		let prio: &'static [f32] = Box::leak(vec![1.0f32; n as usize].into_boxed_slice());
		match infos.iter_mut().find(|i| i.queue_family_index == fam) {
			Some(i) => *i = i.queue_priorities(prio),
			None => infos.push(ash::vk::DeviceQueueCreateInfo::default().queue_family_index(fam).queue_priorities(prio)),
		}
	}
	*PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
}

/// The window's device with the plan's queues only (no NGX): None = the open failed (the caller makes the plain device).
#[cfg(not(target_vendor = "apple"))]
pub(crate) fn open_device(adapter: &wgpu::Adapter, desc: &wgpu::DeviceDescriptor<'_>) -> Option<(wgpu::Device, wgpu::Queue)> {
	use wgpu::hal;
	let fams = families(adapter)?;
	// wgpu-hal opens its game queue on family 0 (adapter.rs open_with_callback).
	if plan(&fams, 0).is_none() {
		eprintln!("fgen: no Vulkan queue for FidelityFX besides the game queue ({fams:?}): today's device, no FSR frame generation on Vulkan");
		return None;
	}
	let cb: Box<hal::vulkan::CreateDeviceCallback<'static>> = Box::new(move |args: hal::vulkan::CreateDeviceCallbackArgs<'_, '_, 'static>| add(args.queue_create_infos, &fams));
	let open = {
		let a = unsafe { adapter.as_hal::<hal::api::Vulkan>() }?;
		match unsafe { a.open_with_callback(desc.required_features, &desc.required_limits, &desc.memory_hints, Some(cb)) } {
			Ok(o) => o,
			Err(e) => {
				eprintln!("fgen: the Vulkan device with frame-generation queues failed ({e:?}): the plain device");
				return None;
			}
		}
	};
	match unsafe { adapter.create_device_from_hal(open, desc) } {
		Ok(dq) => {
			commit();
			Some(dq)
		}
		Err(e) => {
			eprintln!("fgen: create_device_from_hal failed ({e}): the plain device");
			None
		}
	}
}
