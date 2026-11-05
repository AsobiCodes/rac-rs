//! One water mesh for the water pass (`WaterDraw`) and the position target's format.

use super::*;

pub(super) const WPOS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;

/// One water draw of the level pass.
pub(in crate::viewer) struct WaterDraw {
	/// Vertex buffer (`Vertex` layout), index buffer (u32), index count.
	pub vb: wgpu::Buffer,
	pub ib: wgpu::Buffer,
	pub n: u32,
	/// The first index (`range`: a slice of a shared batch).
	pub first: u32,
	/// Its flat plane, if any (`mirror::plane_of`).
	pub plane: super::mirror::Plane,
	/// Its slot in this frame's flow table (`WaterFx::strip_flows`), 0 = still water.
	pub flow: u32,
	/// What drew it (the water log, RAC_WATER_LOG).
	pub kind: &'static str,
	/// [PC] Its box (min x, y, z, max x, y, z) for the wakes and the churn (`Wakes`); none for a slice of a shared batch.
	pub bounds: Option<[f32; 6]>,
}

impl WaterDraw {
	pub(in crate::viewer) fn new(vb: wgpu::Buffer, ib: wgpu::Buffer, n: u32, pos: impl Iterator<Item = [f32; 3]> + Clone) -> WaterDraw {
		let mut b = [f32::MAX, f32::MAX, f32::MAX, f32::MIN, f32::MIN, f32::MIN];
		for p in pos.clone() {
			for k in 0..3 {
				b[k] = b[k].min(p[k]);
				b[k + 3] = b[k + 3].max(p[k]);
			}
		}
		WaterDraw { vb, ib, n, first: 0, plane: super::mirror::plane_of(pos), flow: 0, kind: "", bounds: (b[0] <= b[3]).then_some(b) }
	}

	/// Indices `first..first + n` of a shared batch (`terrain_liquid`): its slot, no mirror plane.
	pub(in crate::viewer) fn range(vb: wgpu::Buffer, ib: wgpu::Buffer, first: u32, n: u32, slot: u32) -> WaterDraw {
		WaterDraw { vb, ib, n, first, plane: None, flow: slot, kind: "", bounds: None }
	}

	/// [PC] Open sea (`swell`): flat water whose box spans at least `SEA_SIZE` (the oceans and Pokitaru's sea, not a pool).
	pub(super) fn sea(&self) -> bool {
		self.plane.is_some_and(|b| (b[3] - b[1]).max(b[4] - b[2]) >= SEA_SIZE) && self.flow == 0 && self.kind != "strip"
	}

	/// [PC] A lake (`waves`): flat still water, spanning at least `LAKE_SIZE` but no open sea.
	pub(super) fn lake(&self) -> bool {
		self.plane.is_some_and(|b| {
			let span = (b[3] - b[1]).max(b[4] - b[2]);
			span >= LAKE_SIZE && span < SEA_SIZE
		}) && self.flow == 0 && self.kind != "strip"
	}

	/// [PC] Its box for the wakes (`Wakes`): water only (not lava, mud or sludge), its flat plane's height when it has one.
	pub(super) fn wake_box(&self) -> Option<[f32; 6]> {
		if self.flow >= LAST_FLOW && self.flow != OPAQUE_SLOT && self.flow != COLD_SLOT {
			return None;
		}
		match (self.plane, self.bounds) {
			(Some(p), _) => Some([p[1], p[2], p[0], p[3], p[4], p[0]]),
			(None, b) => b,
		}
	}

	pub(in crate::viewer) fn kind(mut self, kind: &'static str) -> WaterDraw {
		self.kind = kind;
		self
	}

	/// A flowing river group (`WaterFx::quad_flows`): its flow slot, and no mirror plane (the river slopes).
	pub(in crate::viewer) fn flowing(mut self, slot: u32) -> WaterDraw {
		self.flow = slot;
		self.plane = None;
		self
	}

	/// [PC] Water retail draws milky (Pokitaru's waterfall ponds, `pond_fx_tex`): `OPAQUE_SLOT` (its own colour, no clear
	/// shallows), keeping its mirror plane.
	pub(in crate::viewer) fn opaque(mut self) -> WaterDraw {
		self.flow = OPAQUE_SLOT;
		self
	}

	/// An ocean that is no water (`liquid_slot`: Gaspar's lava, Aridia's mud): its slot, and no mirror plane.
	pub(in crate::viewer) fn liquid(mut self, level: usize) -> WaterDraw {
		if let Some(slot) = liquid_slot(level) {
			self.flow = slot;
			self.plane = None;
		}
		self
	}
}
