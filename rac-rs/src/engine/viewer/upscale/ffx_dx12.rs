//! [PC] DX12 side of the FidelityFX upscale (ffx_frame.rs; lane nups2). wgpu 30's D3D12 command encoder gives no access to its
//! command list, so the dispatch is recorded into lists of our own on the queue wgpu submits to, in the hook between the two
//! halves of the frame's submit (presenter.rs `Cuts::split`): everything before the upscale is on the queue, our list goes next,
//! then the rest of the frame. A ring of `SLOTS` allocator + list pairs with one fence: a slot is reused only after the GPU
//! passed its fence value (`SetEventOnCompletion` with no event blocks until then). A COM ref (`keep`) keeps the textures'
//! ID3D12Resource objects alive, but NOT their memory: wgpu suballocates placed resources and could hand the range to a new
//! texture. What keeps the memory is wgpu's own tracking: the callers (ffx_frame.rs / dlss_frame.rs `dispatch`) transition the
//! same textures again in the encoder after the split, so wgpu holds them until the frame's second half ran, which the queue runs
//! after our list (review R16 F3). DLSS on DX12 (dlss_frame.rs) uses the same ring: `run` takes what records into the list.
//! `Drop` waits for every list (review R16 F4).

use super::ffx_abi as abi;
use std::ffi::c_void;
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D12::*;

/// Lists in flight at most (the frames the presenter can have queued, plus one).
const SLOTS: usize = 4;

/// One frame's work for the hook: the dispatch desc (command list filled in by `run`) and the textures it names.
pub(super) struct Job {
	pub desc: abi::DispatchUpscale,
	pub keep: Vec<ID3D12Resource>,
}

// The desc's raw pointers are the textures in `keep` (and null): moved to the submitting thread with them, used only there.
unsafe impl Send for Job {}

struct Slot {
	alloc: ID3D12CommandAllocator,
	list: ID3D12GraphicsCommandList,
	/// The fence value that marks this slot's last list done (0: never used).
	done: u64,
	keep: Vec<ID3D12Resource>,
}

/// The ring of command lists, the fence and wgpu's queue.
pub(super) struct Lists {
	queue: ID3D12CommandQueue,
	fence: ID3D12Fence,
	value: u64,
	slots: Vec<Slot>,
	next: usize,
}

// D3D12 devices, queues, fences, allocators and lists are free-threaded objects; each list is used by one thread at a time (the
// Mutex around `Lists`).
unsafe impl Send for Lists {}

impl Lists {
	/// The lists on `device` (the wgpu device's ID3D12Device*) for wgpu's D3D12 queue of `queue`.
	pub(super) fn new(device: *mut c_void, queue: &wgpu::Queue) -> Result<Lists, String> {
		let e = |what: &'static str| move |err: windows::core::Error| format!("{what}: {err}");
		let device = unsafe { ID3D12Device::from_raw_borrowed(&device) }.ok_or("no D3D12 device")?.clone();
		let queue = unsafe { queue.as_hal::<wgpu::hal::api::Dx12>() }.ok_or("no D3D12 queue")?.as_raw().clone();
		let fence: ID3D12Fence = unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }.map_err(e("CreateFence"))?;
		let mut slots = Vec::with_capacity(SLOTS);
		for _ in 0..SLOTS {
			let alloc: ID3D12CommandAllocator = unsafe { device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }.map_err(e("CreateCommandAllocator"))?;
			let list: ID3D12GraphicsCommandList = unsafe { device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &alloc, None) }.map_err(e("CreateCommandList"))?;
			unsafe { list.Close() }.map_err(e("Close"))?;
			slots.push(Slot { alloc, list, done: 0, keep: Vec::new() });
		}
		Ok(Lists { queue, fence, value: 0, slots, next: 0 })
	}

	/// On the submitting thread, between the frame's two submits: waits for the next slot, `record`s into its list (the open
	/// ID3D12GraphicsCommandList*) and executes it on the queue, then signals the fence; `keep` stays referenced until it ran.
	pub(super) fn run(&mut self, record: impl FnOnce(*mut c_void) -> Result<(), String>, keep: Vec<ID3D12Resource>) -> Result<(), String> {
		let e = |what: &'static str| move |err: windows::core::Error| format!("{what}: {err}");
		let i = self.next;
		self.next = (i + 1) % self.slots.len();
		let fence = self.fence.clone();
		let s = &mut self.slots[i];
		if s.done > 0 && unsafe { fence.GetCompletedValue() } < s.done {
			unsafe { fence.SetEventOnCompletion(s.done, windows::Win32::Foundation::HANDLE::default()) }.map_err(e("SetEventOnCompletion"))?;
		}
		s.keep.clear();
		unsafe { s.alloc.Reset() }.map_err(e("allocator Reset"))?;
		unsafe { s.list.Reset(&s.alloc, None) }.map_err(e("list Reset"))?;
		let rc = record(s.list.as_raw());
		unsafe { s.list.Close() }.map_err(e("list Close"))?;
		rc?;
		let list: ID3D12CommandList = s.list.cast().map_err(e("ID3D12CommandList"))?;
		unsafe { self.queue.ExecuteCommandLists(&[Some(list)]) };
		self.value += 1;
		s.done = self.value;
		s.keep = keep;
		unsafe { self.queue.Signal(&fence, self.value) }.map_err(e("Signal"))?;
		Ok(())
	}

	/// Blocks until every list executed so far ran (DLSS: the feature's create work before its first frame; `Drop`).
	pub(super) fn wait_all(&self) -> Result<(), String> {
		if self.value > 0 && unsafe { self.fence.GetCompletedValue() } < self.value {
			unsafe { self.fence.SetEventOnCompletion(self.value, windows::Win32::Foundation::HANDLE::default()) }.map_err(|err| format!("SetEventOnCompletion: {err}"))?;
		}
		Ok(())
	}
}

impl Drop for Lists {
	fn drop(&mut self) {
		let _ = self.wait_all();
	}
}
