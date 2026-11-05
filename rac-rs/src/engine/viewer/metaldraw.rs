//! BRIDGE-45 (NOV-35b): the CPU side of the retail metal (chrome) mobys, moved out of viewer.rs (size limit) unchanged.

use super::*;

/// BRIDGE-45 (NOV-35b, moby_format.md 4.1): the metal packets of every class that has them, the chrome and glass textures (clamp
/// sampler; `Draw::texture` 0 = chrome, 1 = glass) and this frame's world batch of the drawn props' metal faces (rebuilt by
/// `Gpu::set_props`, capacity in vertices and indices). The shop icon / popup bolts build theirs at the panel draw.
#[derive(Default)]
pub(crate) struct MetalDraw {
	pub(in super) classes: HashMap<u32, Vec<mobies::MetalPacket>>,
	pub(in super) tex: Option<[wgpu::BindGroup; 2]>,
	pub(in super) props: Option<Batch>,
	pub(in super) cap: (usize, usize),
	/// [PC] perf round 3: the props' metal vertices of the last frame (allocation reused).
	pub(in super) scratch: Vec<Vertex>,
}
